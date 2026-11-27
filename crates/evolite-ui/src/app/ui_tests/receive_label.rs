//! FEAT-0057 П6: «A label for the request» on Receive — typed, it goes into the `dash:` link
//! (UC-6.2); the example inside, who sees it under it (R-6.3); controls and the 61st character
//! not typed (A6.2); leaving the screen clears it (A6.3); Core only (R-6.4).

use super::*;
use crate::screens::Screen;

const LABEL: &str = "A label for the request (optional)";

/// What the app gave the clipboard in the frames after a click.
fn copy_link(kit: &mut Kit) -> Option<String> {
    kit.get_by_role_and_label(Role::Button, "dash: link")
        .click();
    for _ in 0..4 {
        kit.run_steps(1);
        let copied = kit
            .output()
            .platform_output
            .commands
            .iter()
            .find_map(|c| match c {
                egui::OutputCommand::CopyText(t) => Some(t.clone()),
                _ => None,
            });
        if copied.is_some() {
            return copied;
        }
    }
    None
}

fn core_address(kit: &Kit) -> String {
    kit.state()
        .snapshot
        .as_ref()
        .and_then(|s| s.wallet.as_ref())
        .map(|w| w.core_address.clone())
        .expect("a wallet")
}

/// UC-6.2, R-6.3: typed as a person types it, the label is in the link the screen copies,
/// encoded; the warning stays under the field while typing.
#[test]
fn a_typed_label_goes_into_the_link() {
    let (_turn, mut kit) = demo_app("receive", "en");
    settle(&mut kit);
    assert!(has_words(&kit, "0 / 60"), "the count");
    kit.get_by_role_and_label(Role::TextInput, LABEL).click();
    kit.run_steps(2);
    kit.get_by_role_and_label(Role::TextInput, LABEL)
        .type_text("Invoice № 42");
    kit.run_steps(3);
    assert_eq!(kit.state().receive.label, "Invoice № 42");
    assert!(has_words(&kit, "12 / 60"), "the count follows");
    assert!(has_words(
        &kit,
        "The payer and anyone with the link or the QR code will see it"
    ));
    let address = core_address(&kit);
    assert_eq!(
        copy_link(&mut kit).as_deref(),
        Some(format!("dash:{address}?label=Invoice%20%E2%84%96%2042").as_str())
    );
}

/// A6.2: what cannot be in a label is not typed: a bidi mark, a control, the 61st character.
#[test]
fn controls_and_the_sixty_first_character_are_not_typed() {
    let (_turn, mut kit) = demo_app("receive", "en");
    settle(&mut kit);
    kit.get_by_role_and_label(Role::TextInput, LABEL).click();
    kit.run_steps(2);
    kit.get_by_role_and_label(Role::TextInput, LABEL)
        .type_text("pay\u{202E}now");
    kit.run_steps(3);
    assert_eq!(kit.state().receive.label, "paynow");
    kit.state_mut().receive.label = "я".repeat(60);
    kit.run_steps(2);
    kit.get_by_role_and_label(Role::TextInput, LABEL)
        .type_text("ё");
    kit.run_steps(3);
    assert_eq!(kit.state().receive.label.chars().count(), 60);
    assert!(has_words(&kit, "60 / 60"));
}

/// A6.3: leaving Receive clears the label, as the amount (UC-6.3).
#[test]
fn leaving_the_screen_clears_the_label() {
    let (_turn, mut kit) = demo_app("receive", "en");
    settle(&mut kit);
    kit.state_mut().receive.label = "Invoice 7".into();
    kit.run_steps(2);
    kit.state_mut().screen = Screen::Overview;
    kit.run_steps(3);
    kit.state_mut().screen = Screen::Receive;
    kit.run_steps(3);
    assert_eq!(kit.state().receive.label, "");
    let address = core_address(&kit);
    assert_eq!(
        copy_link(&mut kit).as_deref(),
        Some(format!("dash:{address}").as_str())
    );
}

/// R-6.4: the Platform address has no `dash:` link, so no label either.
#[test]
fn a_platform_address_has_no_label() {
    let (_turn, mut kit) = demo_app("receive", "en");
    settle(&mut kit);
    assert!(has_text(&kit, LABEL) || kit.query_all_by_label(LABEL).next().is_some());
    kit.state_mut().receive.target = 1;
    kit.run_steps(3);
    assert!(
        kit.query_all_by_label(LABEL).next().is_none(),
        "Platform: no label"
    );
}

/// A6.4: at 1024 × 768 and 800 × 440, in ru, a 60-character label: the field and the line about
/// who sees it stay in the page's width, and no words are cut or drawn over others.
#[test]
fn the_label_fits_at_1024_and_in_the_smallest_window() {
    for (w, h) in [(1024.0, 768.0), (800.0, 440.0)] {
        let (_turn, mut kit) = demo_app("receive", "ru");
        kit.set_size(egui::vec2(w, h));
        settle(&mut kit);
        kit.state_mut().receive.label = "Счёт № 14 · ".repeat(5);
        kit.run_steps(3);
        let label = crate::i18n::spoken(LABEL);
        let field = kit
            .query_all_by_role_and_label(Role::TextInput, &label)
            .next()
            .map(|n| n.rect())
            .expect("the label's field");
        assert!(field.left() >= 0.0 && field.right() <= w, "{w}: {field:?}");
        let warning =
            crate::i18n::spoken("The payer and anyone with the link or the QR code will see it");
        let under = kit
            .query_all_by(|n| n.value().is_some_and(|v| v == warning))
            .next()
            .map(|n| n.rect())
            .expect("who sees it");
        assert!(under.right() <= w, "{w}: {under:?}");
        let wrong = super::smallest_window::text_trouble(&kit);
        assert!(wrong.is_empty(), "{w}: {wrong:?}");
    }
}

/// r1 (the architect): a bidi mark dropped from the middle of what is typed does not move the
/// cursor on — the next character goes right after the last one kept.
#[test]
fn a_dropped_mark_keeps_the_cursor_after_the_text() {
    let (_turn, mut kit) = demo_app("receive", "en");
    settle(&mut kit);
    kit.get_by_role_and_label(Role::TextInput, LABEL).click();
    kit.run_steps(2);
    kit.get_by_role_and_label(Role::TextInput, LABEL)
        .type_text("abcd");
    kit.run_steps(2);
    kit.key_press(egui::Key::Home);
    kit.key_press(egui::Key::ArrowRight);
    kit.key_press(egui::Key::ArrowRight);
    kit.run_steps(2);
    kit.get_by_role_and_label(Role::TextInput, LABEL)
        .type_text("\u{202E}X");
    kit.run_steps(3);
    assert_eq!(kit.state().receive.label, "abXcd");
    kit.get_by_role_and_label(Role::TextInput, LABEL)
        .type_text("Y");
    kit.run_steps(3);
    assert_eq!(kit.state().receive.label, "abXYcd", "the cursor after X");
}
