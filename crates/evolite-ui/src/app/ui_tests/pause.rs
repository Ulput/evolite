//! FEAT-0002: the pause after too many wrong passwords on the unlock screen and in
//! "Open "…"" — the warning before it, the plate and the disabled button in it, the line after.

use super::*;
use crate::screens::Screen;

fn open_enabled(kit: &Kit, english: &str) -> Option<bool> {
    let name = crate::i18n::spoken(english);
    kit.query_all_by_role(Role::Button)
        .find(|n| n.accesskit_node().label().as_deref() == Some(name.as_str()))
        .map(|n| !n.accesskit_node().is_disabled())
}

fn wrong_tries(kit: &Kit) -> u32 {
    kit.state()
        .snapshot
        .as_ref()
        .and_then(|s| s.wallets.iter().find(|w| w.open))
        .map_or(0, |w| w.wrong_tries)
}

/// Types `password` into the focused password field and presses Enter.
fn type_and_enter(kit: &mut Kit, password: &str) {
    let field = kit
        .query_all_by_role(Role::PasswordInput)
        .next()
        .expect("the password field");
    assert!(
        field.accesskit_node().is_focused(),
        "the field takes the typing"
    );
    field.type_text(password);
    kit.run_steps(2);
    enter(kit);
}

/// The polite live region of the pause: what a screen reader says once.
fn spoken_pause(kit: &Kit) -> Option<String> {
    kit.query_all_by_role(Role::Status)
        .find_map(|n| n.accesskit_node().label())
}

/// AC-A1, A2, D1, D2, A13: wrong passwords on «Welcome back» — a warning after the 3rd and 4th,
/// the plate after the 5th; then the button is off, Enter sends nothing, the typing stays.
#[test]
fn the_fifth_wrong_password_pauses_the_unlock_screen() {
    let (_turn, mut kit) = demo_app("unlock", "en");
    wait_for(&mut kit, "«Welcome back»", |kit| {
        kit.state().screen == Screen::Unlock && has_text(kit, "Welcome back")
    });
    kit.run_steps(2);
    for n in 1..=5 {
        type_and_enter(&mut kit, evolite_core::DEMO_WRONG_PASSWORD);
        wait_for(&mut kit, "the answer", |kit| {
            !kit.state().unlock.waiting && wrong_tries(kit) == n
        });
        kit.run_steps(2);
        let warned = |k: &Kit, left: &str| {
            has_text(
                k,
                &format!("{left} before a pause. Check the keyboard layout and Caps Lock."),
            )
        };
        match n {
            1 | 2 => assert!(has_text(&kit, "Wrong password") && !warned(&kit, "2 attempts left")),
            3 => assert!(has_text(&kit, "Wrong password") && warned(&kit, "2 attempts left")),
            4 => assert!(has_text(&kit, "Wrong password") && warned(&kit, "1 attempt left")),
            _ => {}
        }
        assert_eq!(spoken_pause(&kit).is_some(), n == 5, "try {n}");
    }
    assert!(has_text(&kit, "Too many wrong passwords"));
    assert!(!has_text(&kit, "Wrong password"), "the plate in its place");
    assert!(
        has_text(&kit, "You can try again in 0:30.")
            || has_text(&kit, "You can try again in 0:29.")
    );
    let said = spoken_pause(&kit).unwrap();
    assert!(
        said.starts_with("Too many wrong passwords You can try again in 0:"),
        "{said}"
    );
    // The typing stays; neither the button nor Enter sends it.
    type_and_enter(&mut kit, "typed in the pause");
    kit.run_steps(3);
    assert_eq!(open_enabled(&kit, "Open the wallet"), Some(false));
    assert!(!kit.state().unlock.waiting, "nothing went to the core");
    assert_eq!(kit.state().unlock.password.as_str(), "typed in the pause");
    // The links stay (AC-A8).
    assert!(has_text(&kit, "Forgot the password? · Another wallet"));
    // The pause over (its end moved to now): the plate goes, «Wrong password» does not come
    // back, the next step is named, and the typing can go (AC-D5).
    if let Some(s) = kit.state_mut().snapshot.as_mut() {
        s.wallets.iter_mut().for_each(|w| w.wait_until = None);
    }
    kit.run_steps(3);
    assert!(!has_text(&kit, "Too many wrong passwords"));
    assert!(!has_text(&kit, "Wrong password"));
    assert!(has_text(
        &kit,
        "You can try again. If the password is wrong again, the next pause is 1 min."
    ));
    assert_eq!(open_enabled(&kit, "Open the wallet"), Some(true));
}

/// AC-D5: the pause over — no plate, the button on, and what the next wrong password costs.
#[test]
fn after_the_pause_the_next_one_is_named() {
    let (_turn, mut kit) = demo_app("unlock-wait-end", "en");
    wait_for(&mut kit, "the line after the pause", |kit| {
        has_text(
            kit,
            "You can try again. If the password is wrong again, the next pause is 1 min.",
        )
    });
    assert!(!has_text(&kit, "Too many wrong passwords"));
    assert_eq!(open_enabled(&kit, "Open the wallet"), Some(true));
}

/// AC-A6, D3: «Savings» chosen in a pause has the plate and «Open» off; «Main», chosen, has
/// none and opens with its password.
#[test]
fn another_wallet_in_the_list_has_no_pause() {
    let (_turn, mut kit) = demo_app("wallet-unlock-wait", "en");
    wait_for(&mut kit, "the plate of «Savings»", |kit| {
        has_text(kit, "You can try again in 4:12.")
    });
    assert_eq!(open_enabled(&kit, "Open"), Some(false));
    kit.get_by_role_and_label(Role::RadioButton, "Main").click();
    kit.run_steps(3);
    assert!(!has_text(&kit, "Too many wrong passwords"));
    assert!(spoken_pause(&kit).is_none());
    type_and_enter(&mut kit, "main-wallet-26");
    wait_for(&mut kit, "«Main» open", |kit| {
        !kit.state().unlock.waiting && kit.state().unlock.error.is_none()
    });
}

/// AC-A7, D4: «Open "Savings"» in its pause — the plate instead of the catching-up note,
/// «Open» off, «Cancel» on; the open wallet stays.
#[test]
fn open_another_wallet_in_its_pause() {
    let (_turn, mut kit) = demo_app("wallet-switch-wait", "en");
    wait_for(&mut kit, "the dialog's plate", |kit| {
        has_text(kit, "You can try again in 4:12.")
    });
    assert!(!has_text(
        &kit,
        "After opening, about a minute to catch up: new blocks since it was last open."
    ));
    assert_eq!(open_enabled(&kit, "Open"), Some(false));
    assert_eq!(open_enabled(&kit, "Cancel"), Some(true));
    assert_eq!(kit.state().screen, Screen::Overview);
}

/// AC-D7: in Arabic the countdown reads 0:45, not 45:0.
#[test]
fn the_countdown_reads_left_to_right_in_arabic() {
    let (_turn, mut kit) = demo_app("unlock-wait", "ar");
    wait_for(&mut kit, "the plate", |kit| spoken_pause(kit).is_some());
    let shown = kit
        .query_all(By::new().role(Role::Label))
        .filter_map(|n| n.accesskit_node().value())
        .find(|v| v.contains("45"))
        .expect("the countdown line");
    assert!(shown.contains("0:45") && !shown.contains("45:0"), "{shown}");
}
