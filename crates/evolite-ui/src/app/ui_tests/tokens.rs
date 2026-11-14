//! FEAT-0019 part 4: the tokens screen on the demo core — the menu item, the rows in each state,
//! the refusals before the password, mainnet's explanation.

use super::*;

/// The rect of the menu item named `name` (the label in the menu, the leftmost of that name).
fn menu_item(kit: &Kit, name: &str) -> egui::Rect {
    kit.query_all_by(|n| n.label().as_deref() == Some(name))
        .map(|n| n.rect())
        .min_by(|a, b| a.left().total_cmp(&b.left()))
        .unwrap_or_else(|| panic!("the menu item «{name}»"))
}

/// AC-D1: «Tokens» is in the menu, last, after «Collection».
#[test]
fn tokens_is_the_last_menu_item_after_collection() {
    let (_turn, kit) = demo_app("overview", "en");
    let collection = menu_item(&kit, "Collection");
    let tokens = menu_item(&kit, "Tokens");
    assert!(tokens.top() > collection.top(), "after «Collection»");
    for other in ["Overview", "Send", "Apps"] {
        assert!(
            menu_item(&kit, other).top() < tokens.top(),
            "«{other}» before it"
        );
    }
}

/// AC-D4, AC-T5: the rows of the demo identity, each in its state.
#[test]
fn rows_say_their_state_and_unknown_is_not_zero() {
    let (_turn, mut kit) = demo_app("tokens", "en");
    kit.run_steps(3);
    for name in [
        "Evo Coffee",
        "Workshop Hours",
        "Testnet Gold",
        "Garden Points",
    ] {
        assert!(has_text(&kit, name), "the row «{name}»");
    }
    assert!(has_words(&kit, "found automatically"));
    assert!(has_words(&kit, "paused") && has_words(&kit, "frozen"));
    assert!(has_text(
        &kit,
        "The issuer paused the token — transfers are not available now."
    ));
    assert!(has_text(
        &kit,
        "This token cannot be transferred: its issuer decided so."
    ));
    assert!(has_words(&kit, "something to claim"));
    assert!(
        has_text(&kit, "—"),
        "a balance not read yet is a dash, never 0"
    );
    assert!(has_text(&kit, "125.50"), "Evo Coffee with its two decimals");
    assert!(has_text(&kit, "Keep in mind"));
    // Five rows cannot be sent (paused, frozen, not transferable, not read, zero), two can.
    let sends: Vec<bool> = kit
        .query_all_by_role_and_label(Role::Button, "Send")
        .filter(|n| n.rect().left() > 400.0)
        .map(|n| n.accesskit_node().is_disabled())
        .collect();
    assert_eq!(sends.len(), 7, "a «Send» in every row");
    assert_eq!(sends.iter().filter(|off| !**off).count(), 2, "{sends:?}");
}

/// AC-T3: the send form refuses a Dash address and more than the balance before the password.
#[test]
fn the_send_form_refuses_before_the_password() {
    let (_turn, mut kit) = demo_app("tokens-send", "en");
    kit.run_steps(3);
    assert!(has_text(&kit, "Send a token"));
    if let Some(send) = &mut kit.state_mut().tokens.sending {
        send.to = "yNk2Hc8pWq4mR7vT3sLb9XeJ6uZa5Df1Gh".into();
        send.amount = "5".into();
    }
    kit.run_steps(3);
    assert!(has_text(
        &kit,
        "A token goes to an identity — a Dash address will not do here."
    ));
    assert!(button(&kit, "Check").accesskit_node().is_disabled());
    if let Some(send) = &mut kit.state_mut().tokens.sending {
        send.to = "@alice".into();
        send.amount = "1000".into();
    }
    kit.run_steps(3);
    assert!(has_words(&kit, "More than you have"));
    assert!(button(&kit, "Check").accesskit_node().is_disabled());
    if let Some(send) = &mut kit.state_mut().tokens.sending {
        send.amount = "5".into();
    }
    kit.run_steps(3);
    assert!(!button(&kit, "Check").accesskit_node().is_disabled());
    button(&kit, "Check").click();
    // The name is asked of the network first (R-3), then the review.
    wait_for(&mut kit, "the review", |kit| {
        has_text(kit, "Check the transfer")
    });
    assert!(
        has_words(&kit, "Send to: @alice · "),
        "the name with its short id"
    );
    // The row reads «Amount: 5.00 · Evo Coffee» (kv_row, the words without the isolate marks).
    assert!(
        has_words(&kit, "Amount: 5.00 · Evo Coffee"),
        "the amount with the name outside the row"
    );
}

/// AC-D1, AC-T6: on mainnet the screen only explains; no search, no adding.
#[test]
fn on_mainnet_the_screen_only_explains() {
    let (_turn, mut kit) = demo_app("tokens-mainnet", "en");
    kit.run_steps(3);
    assert!(has_text(&kit, "Tokens work only on testnet for now"));
    assert!(
        kit.query_all_by_role_and_label(Role::Button, "Find my tokens")
            .next()
            .is_none()
    );
    assert!(
        kit.query_all_by_role_and_label(Role::Button, "Add a token")
            .next()
            .is_none()
    );
    assert!(!has_text(&kit, "Evo Coffee"), "no rows");
}

/// AC-D2, AC-D3: the first search, its pause and no network, with «Find my tokens» off while
/// it runs and «Add a token» always on.
#[test]
fn the_search_states_as_drawn() {
    for (preset, says) in [
        (
            "tokens-first-search",
            "Looking for your tokens on the network…",
        ),
        ("tokens-pause", "Looking for your tokens on the network…"),
        (
            "tokens-no-network",
            "Search stopped: no connection to the network",
        ),
        ("tokens-empty", "No tokens found for @satoshi"),
    ] {
        let (_turn, mut kit) = demo_app(preset, "en");
        kit.run_steps(3);
        assert!(has_text(&kit, says), "{preset}: «{says}»");
        assert!(
            kit.query_all_by_role_and_label(Role::Button, "Add a token")
                .all(|b| !b.accesskit_node().is_disabled()),
            "{preset}: adding does not wait for the search"
        );
        let find_off = button(&kit, "Find my tokens")
            .accesskit_node()
            .is_disabled();
        assert_eq!(
            find_off,
            matches!(preset, "tokens-first-search" | "tokens-pause"),
            "{preset}"
        );
    }
    {
        let (_turn, mut kit) = demo_app("tokens-pause", "en");
        kit.run_steps(3);
        assert!(has_words(&kit, "the node asked us to slow down"));
    }
    let (_turn, mut kit) = demo_app("tokens-refresh-failed", "en");
    kit.run_steps(3);
    assert!(has_words(&kit, "Could not find tokens"));
    assert!(has_text(&kit, "Evo Coffee"), "the list stays");
}

/// Design review 1 (BUG-42): the review asks the wallet's password whenever every other spend
/// does, and «Sign and send» waits for it.
#[test]
fn the_review_asks_the_password_as_every_spend() {
    let (_turn, mut kit) = demo_app("tokens-review", "en");
    wait_for(&mut kit, "the review", |kit| {
        has_text(kit, "Check the transfer")
    });
    kit.run_steps(2);
    assert!(
        kit.query_all_by_role(Role::PasswordInput).next().is_some(),
        "the password field"
    );
    assert!(button(&kit, "Sign and send").accesskit_node().is_disabled());
}

/// R-3: a name the network does not know is said under «To», before the password.
#[test]
fn an_unknown_name_is_refused_under_to() {
    let (_turn, mut kit) = demo_app("tokens-send-unknown", "en");
    wait_for(&mut kit, "the refusal", |kit| {
        has_text(kit, "The recipient is not found on the network.")
    });
    assert!(!has_text(&kit, "Check the transfer"), "no review");
}

/// R-3а: a disabled «Send» says why in its row.
#[test]
fn every_disabled_send_says_why() {
    let (_turn, mut kit) = demo_app("tokens", "en");
    kit.run_steps(3);
    assert!(has_text(
        &kit,
        "The balance is 0 — there is nothing to send."
    ));
    assert!(has_text(
        &kit,
        "The balance is not checked yet — you can send after the check."
    ));
}

/// Architect, part 4: «Hide» is not a trap. A hidden found token, found again in «Add a token»,
/// offers «Return to the list», and after the click its row is back.
#[test]
fn a_hidden_token_can_be_returned_from_add() {
    let (_turn, mut kit) = demo_app("tokens", "en");
    kit.run_steps(3);
    let rows_named = |kit: &Kit| {
        kit.query_all_by(|n| n.value().as_deref() == Some("Evo Coffee"))
            .count()
    };
    assert!(rows_named(&kit) > 0, "Evo Coffee in the list");
    kit.query_all_by_role_and_label(Role::Button, "Hide")
        .next()
        .expect("Evo Coffee's «Hide»")
        .click();
    wait_for(&mut kit, "the row hidden", |kit| rows_named(kit) == 0);
    let (add, command) = crate::screens::tokens::AddState::asking("coffee").expect("a query");
    kit.state_mut().tokens.adding = Some(add);
    kit.state_mut().core.send(command);
    wait_for(&mut kit, "«Return to the list»", |kit| {
        kit.query_all_by_role_and_label(Role::Button, "Return to the list")
            .next()
            .is_some()
    });
    button(&kit, "Return to the list").click();
    kit.run_steps(2);
    button(&kit, "Cancel").click();
    wait_for(&mut kit, "the row back", |kit| rows_named(kit) > 0);
}

/// BUG-129: «Hide amounts» hides what a person holds of a token, as it hides DASH: the row's
/// balance, «available» in the form and «Left after» in the review. What they send stays.
#[test]
fn hidden_amounts_hide_token_balances() {
    {
        let (_turn, mut kit) = demo_app("tokens", "en");
        kit.run_steps(3);
        assert!(has_text(&kit, "125.50"), "Evo Coffee's balance at first");
        kit.state_mut().settings.flip_hide_amounts();
        kit.run_steps(3);
        assert!(!has_words(&kit, "125.50"), "the balance hidden in the row");
        assert!(has_text(&kit, "••••"), "«••••» in its place");
    }

    let (_turn, mut kit) = demo_app("tokens-review", "en");
    kit.state_mut().settings.flip_hide_amounts();
    wait_for(&mut kit, "the review", |kit| {
        has_text(kit, "Check the transfer")
    });
    kit.run_steps(2);
    for held in ["125.50", "120.50"] {
        assert!(!has_words(&kit, held), "{held} hidden in the review");
    }
    assert!(
        has_words(&kit, "Left after: •••• · Evo Coffee"),
        "«Left after» masked, the name kept"
    );
    assert!(
        has_words(&kit, "Amount: 5.00 · Evo Coffee"),
        "the amount sent stays"
    );
}

/// BUG-130 (R-1, AC-A6): «Backup» says that the token list stays on this computer, and what a
/// restore brings back of it.
#[test]
fn backup_says_the_token_list_is_kept_here() {
    const TOKENS: &str = "The token list is kept on this computer. After a restore from the phrase, the search finds the tokens with a balance again, hidden ones included: hide them again if needed. Tokens added by hand with a zero balance do not come back — add them again by id.";
    {
        let (_turn, kit) = demo_app("backup", "en");
        assert!(has_text(&kit, TOKENS));
    }
    let (_turn, kit) = demo_app("backup", "ru");
    assert!(has_text(
        &kit,
        "Список токенов хранится только на этом компьютере. После восстановления по фразе поиск снова найдёт токены с балансом — и скрытые тоже, их придётся скрыть заново. Добавленные вручную токены с нулевым балансом не вернутся — добавьте их снова по id."
    ));
}

/// BUG-131: a token added by hand has «Remove» where a found one has «Hide», and after the
/// click its row leaves the list. A found one keeps «Hide».
#[test]
fn a_token_added_by_hand_can_be_removed() {
    let (_turn, mut kit) = demo_app("tokens", "en");
    kit.run_steps(3);
    let count =
        |kit: &Kit, label: &str| kit.query_all_by_role_and_label(Role::Button, label).count();
    let rows_named = |kit: &Kit| {
        kit.query_all_by(|n| n.value().as_deref() == Some("Workshop Hours"))
            .count()
    };
    let (removes, hides) = (count(&kit, "Remove"), count(&kit, "Hide"));
    assert!(
        removes >= 3,
        "a «Remove» on each row added by hand: {removes}"
    );
    assert!(hides >= 1, "found rows keep «Hide»");
    assert!(
        rows_named(&kit) > 0,
        "Workshop Hours, added by hand, in the list"
    );
    kit.query_all_by_role_and_label(Role::Button, "Remove")
        .min_by(|a, b| a.rect().top().total_cmp(&b.rect().top()))
        .expect("the first «Remove»")
        .click();
    wait_for(&mut kit, "the row removed", |kit| rows_named(kit) == 0);
    assert_eq!(count(&kit, "Remove"), removes - 1);
    assert_eq!(count(&kit, "Hide"), hides);
}

/// BUG-132: before the network's count is known, the first search says what it counts.
#[test]
fn the_first_search_says_what_it_counts() {
    let (_turn, mut kit) = demo_app("tokens-first-count", "en");
    kit.run_steps(3);
    assert!(has_text(&kit, "Looking for your tokens on the network…"));
    assert!(has_text(&kit, "1,840 network contracts read"));
}
