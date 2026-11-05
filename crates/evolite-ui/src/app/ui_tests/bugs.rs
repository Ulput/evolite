//! One test per bug whose UI half kittest can show: what the person does, what must follow.

use super::*;
use crate::screens::Screen;
use crate::screens::settings::NodesTab;

/// BUG-18: Settings → Network and nodes → a click on the words «Core nodes» (not on the
/// chevron) opens the nodes window on its Core tab.
#[test]
fn settings_core_nodes_opens_from_a_click_on_its_words() {
    let (_turn, mut kit) = demo_app("settings", "en");
    assert_eq!(kit.state().settings.nodes, None);
    words(&kit, "Core nodes").click();
    kit.run_steps(2);
    assert_eq!(kit.state().settings.nodes, Some(NodesTab::Core));
}

/// BUG-18 r3: the rows «Core nodes · N» and «Platform (DAPI) · N» are one Tab stop each — Tab
/// from the first lands on the second, Shift+Tab comes back — and Enter on a row opens its tab
/// of the nodes window.
///
/// Failed on dev d6cc997: Tab went «Testnet» → focus lost → the sidebar, until the row got its
/// own id (`screens/settings/parts.rs`, `row`).
#[test]
fn settings_node_rows_are_tab_stops_and_open_with_enter() {
    let (_turn, mut kit) = demo_app("settings", "en");
    tab_to(&mut kit, "Core nodes ·");
    tab(&mut kit, false);
    let next = focused(&kit).unwrap_or_default();
    assert!(
        next.starts_with("Platform (DAPI) ·"),
        "Tab went to «{next}»"
    );
    enter(&mut kit);
    assert_eq!(kit.state().settings.nodes, Some(NodesTab::Platform));

    kit.state_mut().settings.nodes = None;
    kit.run_steps(2);
    tab_to(&mut kit, "Platform (DAPI) ·");
    tab(&mut kit, true);
    let back = focused(&kit).unwrap_or_default();
    assert!(
        back.starts_with("Core nodes ·"),
        "Shift+Tab went to «{back}»"
    );
    enter(&mut kit);
    assert_eq!(kit.state().settings.nodes, Some(NodesTab::Core));
}

/// BUG-17: while a wallet is being added, a click on the words of «‹ Back to “Main”» (not on
/// the arrow) goes back to that wallet's unlock screen.
#[test]
fn back_to_the_wallet_answers_a_click_on_its_words() {
    let (_turn, mut kit) = demo_app("wallet-menu", "en");
    button(&kit, "Add wallet").click();
    kit.run_steps(2);
    wait_for(&mut kit, "room for the new wallet", |kit| {
        kit.state()
            .snapshot
            .as_ref()
            .is_some_and(|s| s.adding_wallet)
            && kit.state().screen == Screen::Welcome
    });
    kit.run_steps(2);
    click_words_of(&mut kit, "Back to “Main”");
    wait_for(&mut kit, "way back to “Main”", |kit| {
        kit.state()
            .snapshot
            .as_ref()
            .is_some_and(|s| !s.adding_wallet)
    });
    kit.run_steps(2);
    assert_eq!(kit.state().screen, Screen::Unlock);
    assert!(kit.query_by_label("Back to “Main”").is_none());
}

/// BUG-1: on «Which wallet to open?» in Russian the wallets are called as their owner named
/// them — «Main», «Savings» — never translated as words of the interface. (Only the names a
/// screen reader hears: the painted words are not in the tree.)
#[test]
fn wallet_names_in_the_unlock_list_are_not_translated() {
    let (_turn, mut kit) = demo_app("wallet-unlock", "ru");
    wait_for(&mut kit, "the wallets to open", |kit| {
        kit.query_by_label("Savings").is_some()
    });
    for name in ["Main", "Savings"] {
        let row = kit.get_by_role_and_label(Role::RadioButton, name);
        assert!(!row.accesskit_node().is_disabled(), "{name}");
    }
    for translated in ["Основной", "Сбережения"] {
        assert!(
            kit.query_all_by(|n| n.label().is_some_and(|l| l.contains(translated)))
                .next()
                .is_none(),
            "«{translated}» in the list"
        );
    }
}

/// BUG-20: in an operation's card «Copy txid» and «Open in the explorer» stand apart, at least
/// 12 points between them (the look of their frames is for the screenshots).
#[test]
fn the_txid_buttons_of_an_operation_card_stand_apart() {
    let (_turn, mut kit) = demo_app("activity-card", "en");
    wait_for(&mut kit, "the card's txid", |kit| {
        kit.query_by_label("Open in the explorer").is_some()
    });
    let copy = button(&kit, "Copy txid").rect();
    let open = button(&kit, "Open in the explorer").rect();
    let between = gap(copy, open);
    assert!(between >= 12.0 - 0.5, "{between} points between them");
}

/// BUG-16 (its UI half; the real core's wake is `real::tests`): a wrong password and Enter —
/// «Wrong password» comes on screen with no click or other input after the Enter.
#[test]
fn a_wrong_password_and_enter_show_the_error_at_once() {
    let (_turn, mut kit) = demo_app("wallet-unlock", "en");
    // The preset types a password; from an empty field, as a person starts.
    kit.state_mut().unlock.password.clear();
    kit.run_steps(3);
    let field = kit
        .query_all_by_role(Role::PasswordInput)
        .next()
        .expect("the password field");
    assert!(
        field.accesskit_node().is_focused(),
        "the field takes the typing"
    );
    field.type_text(evolite_core::DEMO_WRONG_PASSWORD);
    kit.run_steps(2);
    enter(&mut kit);
    wait_for(&mut kit, "«Wrong password»", |kit| {
        has_text(kit, "Wrong password")
    });
    assert_eq!(kit.state().screen, Screen::Unlock);
    assert!(kit.state().unlock.password.is_empty(), "wiped once sent");
}

/// The amount fields of a screen, as the tree has them: each field's text.
fn amounts(kit: &Kit) -> Vec<String> {
    kit.query_all_by_role(Role::TextInput)
        .filter_map(|n| n.accesskit_node().value())
        .filter(|v| crate::format::parse_amount(v).is_some())
        .collect()
}

/// An enabled button a screen reader calls `english`, in the language on screen.
fn enabled(kit: &Kit, english: &str) -> bool {
    let name = crate::i18n::spoken(english);
    kit.query_all_by_role(Role::Button).any(|n| {
        n.accesskit_node().label().as_deref() == Some(&name) && !n.accesskit_node().is_disabled()
    })
}

/// BUG-5, BUG-6: in the «1,234.56» number format (English, Arabic) the suggested amounts are
/// written in that format and taken: the field holds them, the operation's button is on.
#[test]
fn suggested_amounts_are_taken_in_english_and_arabic() {
    use crate::format::{input_text, parse_amount, short_number};
    for lang in ["en", "ar"] {
        for (preset, duffs, action) in [
            ("top-up", 10_000_000, None),
            ("shield-core", 50_000_000, Some("Shield")),
            ("create-identity", 30_000_000, None),
            ("send", 25_000_000, Some("Check and send")),
        ] {
            // What the screen shows and allows follows the demo core's answers, which come on
            // its own thread: waited for, not read at once (the architect's run under load).
            let (_turn, mut kit) = demo_app(preset, lang);
            let what = format!("{preset} {lang}: {} in a field", input_text(duffs));
            wait_for(&mut kit, &what, |kit| {
                amounts(kit).iter().any(|v| parse_amount(v) == Some(duffs))
            });
            if let Some(action) = action {
                let what = format!("{preset} {lang}: «{action}» on");
                wait_for(&mut kit, &what, |kit| enabled(kit, action));
            }
        }
        // The top-up's button says the amount only once it is taken.
        {
            let (_turn, mut kit) = demo_app("top-up", lang);
            let says = |kit: &Kit, duffs| {
                let amount = input_text(duffs);
                kit.query_all_by_role(Role::Button).any(|n| {
                    let a = n.accesskit_node();
                    !a.is_disabled() && a.label().is_some_and(|l| l.contains(&amount))
                })
            };
            wait_for(
                &mut kit,
                &format!("top-up {lang}: «Top up by 0.1»"),
                |kit| says(kit, 10_000_000),
            );
            // The pill «0.5» puts 0.5 in the field, and it is taken too.
            kit.get_by_label(&short_number(50_000_000)).click();
            kit.run_steps(3);
            wait_for(
                &mut kit,
                &format!("top-up {lang}: 0.5 from the pill"),
                |kit| {
                    amounts(kit)
                        .iter()
                        .any(|v| parse_amount(v) == Some(50_000_000))
                },
            );
            wait_for(
                &mut kit,
                &format!("top-up {lang}: «Top up by 0.5»"),
                |kit| says(kit, 50_000_000),
            );
        }
        // `send-confirm`: the last look before signing opens.
        let (_turn, mut kit) = demo_app("send-confirm", lang);
        let name = crate::i18n::spoken("Sign and send");
        wait_for(&mut kit, "«Sign and send»", |kit| {
            kit.query_by_label(&name).is_some()
        });
    }
}
