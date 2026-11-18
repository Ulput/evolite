//! SEC-3 and SEC-8 (the phrase review, В1 and П8): the words of the phrase and a password typed
//! reach no IME, and egui's memory — which eframe writes to `app.ron` every 30 s — keeps none of
//! them.

use super::*;

/// The words typed into the restore's fields, and a password into the unlock's.
const WORDS: [&str; 3] = ["zebra", "quantum", "oyster"];
const PASSWORD: &str = "Orchid-Lantern-77";

/// egui's memory as eframe writes it to `app.ron`.
fn persisted(kit: &Kit) -> String {
    kit.ctx
        .memory(|m| ron::to_string(m).expect("egui's memory serializes"))
}

/// Clicks the field `node` gives (so it has the focus), types `text` into it.
fn type_into(kit: &mut Kit, role: Role, label: &str, text: &str) {
    kit.get_by_role_and_label(role, label).click();
    kit.run_steps(2);
    let field = kit.get_by_role_and_label(role, label);
    assert!(field.is_focused(), "{label} has the focus");
    field.type_text(text);
    kit.run_steps(2);
}

/// SEC-3: a focused phrase-word field takes egui's IME request back, so egui-winit switches the
/// window's IME off; an ordinary field keeps it.
#[test]
fn a_focused_phrase_word_asks_for_no_ime() {
    let (_turn, mut kit) = demo_app("restore", "en");
    kit.run_steps(3);
    type_into(&mut kit, Role::TextInput, "Word no. 1", "zeb");
    assert!(
        kit.output().platform_output.ime.is_none(),
        "no IME for a word"
    );
    drop(kit);
    drop(_turn);
    let (_turn, mut kit) = demo_app("activity", "en");
    kit.run_steps(3);
    type_into(
        &mut kit,
        Role::TextInput,
        "Address, name, amount, note",
        "ab",
    );
    assert!(
        kit.output().platform_output.ime.is_some(),
        "the search keeps its IME"
    );
}

/// SEC-3, the password half (review M1, S3): the unlock's password field, focused and typed
/// into, asks for no IME, and the window takes a password's input scope; once the focus leaves
/// it, the scope goes back to the default. Every password field is a `text_field(Password)`.
#[test]
fn a_focused_password_asks_for_no_ime_and_its_scope_goes_with_the_focus() {
    let (_turn, mut kit) = demo_app("unlock", "en");
    kit.run_steps(3);
    let field = kit
        .query_all_by_role(Role::PasswordInput)
        .next()
        .expect("the password field");
    assert!(field.is_focused(), "the unlock's field takes the typing");
    field.type_text("Orch");
    kit.run_steps(2);
    assert!(
        kit.output().platform_output.ime.is_none(),
        "no IME for a password"
    );
    assert_eq!(
        crate::secret_input::applied(),
        Some(true),
        "a password's input scope while it has the focus"
    );
    kit.key_press(Key::Tab);
    kit.run_steps(2);
    assert!(
        kit.query_all_by_role(Role::PasswordInput)
            .all(|f| !f.is_focused()),
        "the focus left the password"
    );
    assert_eq!(
        crate::secret_input::applied(),
        Some(false),
        "the default input scope again"
    );
}

/// SEC-8 (П8): words of the phrase and a password typed leave nothing of themselves in what
/// eframe persists: egui keeps only a field's cursor, never its text.
#[test]
fn app_ron_keeps_no_phrase_word_or_password() {
    let (_turn, mut kit) = demo_app("restore", "en");
    kit.run_steps(3);
    for (i, word) in WORDS.iter().enumerate() {
        type_into(
            &mut kit,
            Role::TextInput,
            &format!("Word no. {}", i + 1),
            word,
        );
    }
    let memory = persisted(&kit);
    for word in WORDS {
        assert!(!memory.contains(word), "«{word}» in egui's memory");
    }
    drop(kit);
    drop(_turn);
    let (_turn, mut kit) = demo_app("unlock", "en");
    kit.run_steps(3);
    let field = kit
        .query_all_by_role(Role::PasswordInput)
        .next()
        .expect("the password field");
    assert!(field.is_focused(), "the unlock's field takes the typing");
    field.type_text(PASSWORD);
    kit.run_steps(2);
    assert!(
        !persisted(&kit).contains(PASSWORD),
        "the password in egui's memory"
    );
}
