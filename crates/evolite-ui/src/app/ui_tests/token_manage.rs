//! FEAT-0019 T-3: the token page and its «Management» on the demo core — what is drawn and what
//! is not (R-15), the group line with no button (T-3.9), the ticks of what cannot be undone
//! (R-23), the locked settings, and the demo never pretending to act.

use super::*;

fn page(kit: &mut Kit) {
    wait_for(kit, "the token page", |kit| has_text(kit, "Management"));
}

/// AC-D8, AC-T13: a click on the name opens the page; the actions taken alone are buttons, the
/// one only a group takes is a line with no button.
#[test]
fn the_name_opens_the_page_with_what_this_identity_may_do() {
    let (_turn, mut kit) = demo_app("tokens", "en");
    kit.run_steps(3);
    button(&kit, "Evo Coffee").click();
    page(&mut kit);
    for name in ["Mint", "Burn", "Freeze", "Pause", "Price", "Unfreeze"] {
        assert!(
            kit.query_all_by_role_and_label(Role::Button, name)
                .next()
                .is_some(),
            "«{name}»"
        );
    }
    assert!(has_text(
        &kit,
        "Destroy frozen funds · This needs signatures from the group: 2 of 3"
    ));
    assert!(
        kit.query_all_by_role_and_label(Role::Button, "Destroy")
            .next()
            .is_none(),
        "no button for a group's action"
    );
    assert!(has_words(&kit, "Can never change"), "the names: locked");
}

/// R-23, AC-D9: a burn waits for «I understand»; more than the balance is refused before the
/// password.
#[test]
fn a_burn_waits_for_the_tick_and_the_balance() {
    let (_turn, mut kit) = demo_app("tokens-burn", "en");
    wait_for(&mut kit, "the burn window", |kit| {
        has_text(kit, "Burn Evo Coffee")
    });
    if let Some(form) = &mut kit.state_mut().tokens.issuer {
        form.amount = "1".into();
    }
    kit.run_steps(3);
    assert!(
        button(&kit, "Sign and burn").accesskit_node().is_disabled(),
        "no tick yet"
    );
    if let Some(form) = &mut kit.state_mut().tokens.issuer {
        form.understood = true;
    }
    kit.run_steps(3);
    assert!(!button(&kit, "Sign and burn").accesskit_node().is_disabled());
    if let Some(form) = &mut kit.state_mut().tokens.issuer {
        form.amount = "1000".into();
    }
    kit.run_steps(3);
    assert!(has_words(&kit, "More than you have"));
    assert!(button(&kit, "Sign and burn").accesskit_node().is_disabled());
}

/// The demo core never pretends: a pause signed is refused, and the window says so.
#[test]
fn the_demo_does_not_pretend_to_pause() {
    let (_turn, mut kit) = demo_app("tokens-suspend", "en");
    wait_for(&mut kit, "the pause window", |kit| {
        has_text(kit, "Pause Evo Coffee")
    });
    if let Some(form) = &mut kit.state_mut().tokens.issuer {
        form.understood = true;
    }
    kit.run_steps(3);
    button(&kit, "Sign and stop").click();
    wait_for(&mut kit, "the refusal", |kit| has_words(kit, "Not done"));
}

/// AC-T14, the designer's M7: the window opens on who it is now, with no tick; handing the rule
/// to «nobody» asks for it.
#[test]
fn handing_a_rule_to_nobody_asks_for_the_tick() {
    let (_turn, mut kit) = demo_app("tokens-setting", "en");
    wait_for(&mut kit, "the settings window", |kit| {
        has_text(kit, "Settings of Evo Coffee")
    });
    kit.run_steps(3);
    assert!(
        !has_words(&kit, "I understand: this cannot be changed back"),
        "not for the contract owner"
    );
    if let Some(form) = &mut kit.state_mut().tokens.issuer {
        form.choice = 0;
    }
    kit.run_steps(3);
    assert!(has_words(&kit, "I understand: this cannot be changed back"));
    assert!(button(&kit, "Sign and save").accesskit_node().is_disabled());
}

/// The designer's M1: the page is a screen of its own, not a window over the list — «‹ Tokens»
/// above its name, Tokens lit; the link and Esc lead back to Tokens (BUG-169's one rule).
#[test]
fn the_page_is_a_screen_that_goes_back_to_tokens() {
    use crate::screens::Screen;
    let (_turn, mut kit) = demo_app("tokens", "en");
    kit.run_steps(3);
    button(&kit, "Evo Coffee").click();
    page(&mut kit);
    assert_eq!(kit.state().screen, Screen::TokenPage);
    let link = |kit: &Kit| {
        kit.query_by_role_and_label(Role::Link, "Back: Tokens")
            .is_some()
    };
    assert!(link(&kit), "«‹ Tokens»");
    kit.query_by_role_and_label(Role::Link, "Back: Tokens")
        .expect("the link")
        .click();
    wait_for(&mut kit, "back on Tokens", |k| {
        k.state().screen == Screen::Tokens
    });
    kit.run_steps(2);
    assert!(!has_text(&kit, "Management"), "no page over the list");
    button(&kit, "Evo Coffee").click();
    page(&mut kit);
    kit.key_press(egui::Key::Escape);
    wait_for(&mut kit, "Esc: back on Tokens", |k| {
        k.state().screen == Screen::Tokens
    });
}

/// A preset that opens the page has no place remembered: it still goes back to Tokens.
#[test]
fn the_page_opened_directly_goes_back_to_tokens() {
    use crate::screens::Screen;
    let (_turn, mut kit) = demo_app("tokens-page", "en");
    page(&mut kit);
    assert!(
        kit.query_by_role_and_label(Role::Link, "Back: Tokens")
            .is_some()
    );
    kit.key_press(egui::Key::Escape);
    wait_for(&mut kit, "back on Tokens", |k| {
        k.state().screen == Screen::Tokens
    });
}

/// Must 1: the demo's limit 1 000 000.00 and 125 500.00 out — 874 500.00 more may be minted,
/// a cent over and «Sign» is off, the line in red says how much is left.
#[test]
fn a_mint_over_the_limit_cannot_be_signed() {
    let (_turn, mut kit) = demo_app("tokens-mint", "en");
    wait_for(&mut kit, "the mint window", |kit| {
        has_text(kit, "Mint Evo Coffee")
    });
    let amount = |kit: &mut Kit, n: &str| {
        if let Some(form) = &mut kit.state_mut().tokens.issuer {
            form.amount = n.into();
        }
        kit.run_steps(3);
    };
    amount(&mut kit, "874500");
    assert!(
        !button(&kit, "Sign and mint").accesskit_node().is_disabled(),
        "fits"
    );
    amount(&mut kit, "874500.01");
    assert!(has_words(&kit, "The supply limit allows"));
    assert!(
        button(&kit, "Sign and mint").accesskit_node().is_disabled(),
        "over the limit"
    );
}

/// Must 2: no choice of where new tokens go and no destination in the rule — no «Mint», and the
/// page says why.
#[test]
fn no_mint_without_a_destination() {
    let (_turn, mut kit) = demo_app("tokens-page", "en");
    page(&mut kit);
    if let Some(p) = kit
        .state_mut()
        .snapshot
        .as_mut()
        .and_then(|s| s.wallet.as_mut())
        .and_then(|w| w.tokens.page.as_mut())
    {
        p.choose_destination = false;
        p.new_tokens_to = None;
    }
    kit.run_steps(3);
    assert!(has_words(&kit, "Minting is impossible"));
    assert!(
        kit.query_all_by_role_and_label(Role::Button, "Mint")
            .next()
            .is_none(),
        "no «Mint»"
    );
    assert!(
        kit.query_all_by_role_and_label(Role::Button, "Burn")
            .next()
            .is_some(),
        "the rest stays"
    );
}

/// The designer's ar review: the group's line is one sentence put in reading order once — its
/// action's words were translated and turned first, then turned again, and came out reversed.
#[test]
fn the_groups_line_reads_in_order_in_arabic() {
    let (_turn, mut kit) = demo_app("tokens-page", "ar");
    wait_for(&mut kit, "the page", |kit| {
        kit.state()
            .snapshot
            .as_ref()
            .is_some_and(|s| s.wallet.as_ref().is_some_and(|w| w.tokens.page.is_some()))
    });
    kit.run_steps(3);
    // The action in Arabic words, in reading order, and nothing of the line left in English.
    let action = crate::i18n::t("Destroy frozen funds").into_owned();
    assert!(has_words(&kit, &action), "{action:?}");
    assert!(!has_words(&kit, "signatures"), "the line in Arabic");
    assert!(
        !has_words(&kit, &crate::i18n::visual(&action)),
        "not turned twice"
    );
}

/// The designer's M1: where the owner destroys frozen funds alone, «Destroy» stands beside
/// «Unfreeze» in the frozen row.
#[test]
fn the_frozen_row_offers_destroy_beside_unfreeze() {
    let (_turn, mut kit) = demo_app("tokens-page-destroy", "en");
    page(&mut kit);
    kit.run_steps(3);
    for name in ["Unfreeze", "Destroy"] {
        assert!(
            kit.query_all_by_role_and_label(Role::Button, name)
                .next()
                .is_some(),
            "«{name}»"
        );
    }
}

/// The designer's M7 and the architect's r2: a who-window is titled «Who may mint», says «was …,
/// becomes …» once something else is chosen, and «It is set this way already» while not.
#[test]
fn a_setting_says_what_it_was_and_becomes() {
    let (_turn, mut kit) = demo_app("tokens-setting", "en");
    wait_for(&mut kit, "the settings window", |kit| {
        has_text(kit, "Settings of Evo Coffee")
    });
    kit.run_steps(3);
    assert!(has_text(&kit, "Who may mint"));
    assert!(has_words(&kit, "It is set this way already."));
    assert!(button(&kit, "Sign and save").accesskit_node().is_disabled());
    if let Some(form) = &mut kit.state_mut().tokens.issuer {
        form.choice = 0;
    }
    kit.run_steps(3);
    assert!(has_words(
        &kit,
        "was the contract owner — you, becomes nobody"
    ));
    assert!(!has_words(&kit, "It is set this way already."));
}

/// Token names: the window starts from the token's names, a row a language; their own is no
/// change («It is set this way already»); a name Platform would refuse says why under the rows
/// and «Sign and save» stays off; a new language is a change.
#[test]
fn the_names_window_edits_the_names_platform_would_take() {
    let (_turn, mut kit) = demo_app("tokens-names", "en");
    wait_for(&mut kit, "the names window", |kit| {
        kit.state()
            .tokens
            .issuer
            .as_ref()
            .is_some_and(|f| f.names_read)
    });
    kit.run_steps(3);
    let names = |kit: &Kit| {
        kit.state()
            .tokens
            .issuer
            .as_ref()
            .map(|f| f.names.iter().map(|n| n.lang.clone()).collect::<Vec<_>>())
            .unwrap_or_default()
    };
    assert_eq!(names(&kit), vec!["en", "ru"], "the token's own");
    assert!(has_words(&kit, "It is set this way already."));
    assert!(
        !button(&kit, "Add a language")
            .accesskit_node()
            .is_disabled(),
        "«Add a language»"
    );
    // A space: Platform would refuse it.
    if let Some(form) = &mut kit.state_mut().tokens.issuer {
        form.names[0].one = "Evo Coffee".into();
    }
    kit.run_steps(3);
    assert!(has_words(&kit, "has a space: a name has none."));
    assert!(button(&kit, "Sign and save").accesskit_node().is_disabled());
    // A new language instead: a change, «was … becomes».
    if let Some(form) = &mut kit.state_mut().tokens.issuer {
        form.names[0].one = "Coffee".into();
        form.names.push(evolite_api::tokens::TokenName {
            lang: "de".into(),
            one: "Kaffee".into(),
            many: "Kaffees".into(),
            capitalize: true,
        });
    }
    kit.run_steps(3);
    assert!(!has_words(&kit, "It is set this way already."));
    assert!(!has_words(&kit, "has a space"));
    assert!(has_words(&kit, "becomes en Coffee / Coffees, ru"));
}
