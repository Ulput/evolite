//! Tab stops and their names (BUG-40, BUG-41); the Tab round over every screen is `tab_sweep`.

use super::*;
use egui::accesskit::Toggled;

/// Whether the checkbox a screen reader calls `name` is ticked.
fn ticked(kit: &Kit, name: &str) -> bool {
    kit.get_by_role_and_label(Role::CheckBox, name)
        .accesskit_node()
        .toggled()
        == Some(Toggled::True)
}

/// BUG-40 (found by developer 2): a click on the words of a checkbox ticks it, as a click on its
/// box does — «The phrase is written down and checked» before deleting a wallet.
#[test]
fn the_words_of_a_checkbox_tick_it() {
    let (_turn, mut kit) = demo_app("delete", "en");
    let name = "The phrase is written down and checked";
    assert!(!ticked(&kit, name), "the dialog opens unticked");
    words(&kit, name).click();
    kit.run_steps(2);
    assert!(ticked(&kit, name), "a click on the words ticks it");
    words(&kit, name).click();
    kit.run_steps(2);
    assert!(!ticked(&kit, name), "and a second one unticks it");
}

/// BUG-40: Tab stops on a checkbox and goes on from it; the focus is never lost there.
#[test]
fn tab_goes_on_from_a_checkbox() {
    let (_turn, mut kit) = demo_app("delete", "en");
    let name = "The phrase is written down and checked";
    tab_to(&mut kit, name);
    tab(&mut kit, false);
    assert!(focused(&kit).is_some(), "the focus is lost after «{name}»");
}

/// BUG-41: a dialog's × is «Закрыть» to a screen reader, and a field is called by its label.
#[test]
fn the_cross_and_the_fields_have_names() {
    let (_turn, mut kit) = demo_app("change-password", "ru");
    kit.get_by_role_and_label(Role::Button, "Закрыть");
    kit.get_by_role_and_label(Role::PasswordInput, "Текущий пароль");
    kit.get_by_role_and_label(Role::PasswordInput, "Новый пароль");
    // Tab comes to them by those names.
    tab_to(&mut kit, "Закрыть");
    tab_to(&mut kit, "Текущий пароль");
}

/// BUG-41: a field without a label above is called by what it is for: the words of the phrase,
/// the search in History.
#[test]
fn fields_without_a_label_have_names_too() {
    let (_turn, kit) = demo_app("restore", "en");
    kit.get_by_role_and_label(Role::TextInput, "Word no. 1");
    kit.get_by_role_and_label(Role::TextInput, "Word no. 12");
    drop(kit);
    drop(_turn);
    let (_turn, kit) = demo_app("activity", "en");
    kit.get_by_role_and_label(Role::TextInput, "Address, name, amount, note");
}
