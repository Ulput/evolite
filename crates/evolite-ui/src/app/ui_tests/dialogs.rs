//! Branch D: dialogs and fields (BUG-39 items 2, 3, 4, 14).

use super::*;

/// The «Title» field of «Wallet "…"» with `name` in it, focused by a click.
fn rename_field(name: &str) -> (MutexGuard<'static, ()>, Kit) {
    let (turn, mut kit) = demo_app("wallet-rename", "en");
    kit.state_mut()
        .settings
        .rename
        .as_mut()
        .expect("the rename dialog")
        .name = name.into();
    kit.run_steps(2);
    kit.get_by_role(Role::TextInput).click();
    kit.run_steps(2);
    (turn, kit)
}

fn renamed(kit: &Kit) -> String {
    kit.state()
        .settings
        .rename
        .as_ref()
        .expect("still open")
        .name
        .clone()
}

/// BUG-39 №14 (not reproduced here: a guard): Ctrl+A in the wallet's name field takes the whole
/// name, so typing replaces it.
#[test]
fn ctrl_a_takes_the_whole_wallet_name() {
    let (_turn, mut kit) = rename_field("My old savings");
    kit.key_press_modifiers(egui::Modifiers::COMMAND, Key::A);
    kit.run_steps(2);
    kit.get_by_role(Role::TextInput).type_text("X");
    kit.run_steps(2);
    assert_eq!(renamed(&kit), "X");
}

/// BUG-39 №14 (not reproduced here: a guard): a triple click in the wallet's name field takes
/// the whole name, not one word.
#[test]
fn a_triple_click_takes_the_whole_wallet_name() {
    let (_turn, mut kit) = rename_field("My old savings");
    // On the words, three presses and releases 50 ms apart, each in its own frame, as a hand
    // clicks (a kittest step alone is 0.25 s: too slow for a triple click).
    let at = kit.get_by_role(Role::TextInput).rect().left_center() + egui::vec2(20.0, 0.0);
    kit.hover_at(at);
    kit.step();
    let mut time = kit.ctx.input(|i| i.time);
    for pressed in [true, false, true, false, true, false] {
        time += 0.05;
        kit.input_mut().time = Some(time);
        kit.input_mut().events.push(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        kit.step();
    }
    kit.run_steps(2);
    kit.get_by_role(Role::TextInput).type_text("X");
    kit.run_steps(2);
    assert_eq!(renamed(&kit), "X");
}

/// BUG-39 №2: a click on the identity's short ID itself copies it, as a click on the copy icon
/// does, and says «copied».
#[test]
fn a_click_on_the_identity_id_copies_it() {
    let (_turn, mut kit) = demo_app("identity", "en");
    let id = kit
        .state()
        .snapshot
        .as_ref()
        .and_then(|s| s.wallet.as_ref())
        .and_then(|w| w.identity.as_ref())
        .expect("the demo identity")
        .id
        .clone();
    let short = crate::text::short_id(&id);
    // The pointer on the words, not an AccessKit action on the label.
    let at = words(&kit, &short).rect().center();
    kit.hover_at(at);
    kit.step();
    kit.drag_at(at);
    kit.step();
    kit.drop_at(at);
    kit.run_steps(2);
    assert!(kit.state().identity.copied_at.is_some(), "the ID is copied");
    assert!(has_text(&kit, "copied"), "and the row says so");
}

/// BUG-39 №3: a line of error that comes and goes under a field does not move the dialog's top,
/// so the field stays under the pointer; the dialog grows downwards.
#[test]
fn an_error_line_does_not_move_the_dialog() {
    let (_turn, mut kit) = demo_app("wallet-rename", "en");
    kit.run_steps(3);
    let before = words(&kit, "Title").rect().top();
    for error in [Some("No longer than 40 characters".to_string()), None] {
        kit.state_mut()
            .settings
            .rename
            .as_mut()
            .expect("the rename dialog")
            .error = error.clone();
        kit.run_steps(3);
        let now = words(&kit, "Title").rect().top();
        assert!(
            (now - before).abs() < 0.5,
            "«Title» moved from {before} to {now} (error {error:?})"
        );
    }
}

/// BUG-39 №4: «ab» in «Add a contact» says why «Send the request» stays off, as «Send» does.
#[test]
fn a_two_letter_contact_name_says_why() {
    let (_turn, mut kit) = demo_app("add-contact", "en");
    kit.state_mut()
        .contacts
        .adding
        .as_mut()
        .expect("the add-contact dialog")
        .name = "ab".into();
    kit.run_steps(3);
    assert!(has_text(
        &kit,
        "This is not a Dash name: 3–63 letters, digits and hyphens"
    ));
    assert!(
        button(&kit, "Send the request")
            .accesskit_node()
            .is_disabled()
    );
}
