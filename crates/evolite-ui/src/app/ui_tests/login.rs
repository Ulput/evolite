//! BUG-53: the language and the theme can be changed on the login screens, before the
//! password — Welcome, «Which wallet to open?» and «Welcome back».

use super::*;
use crate::screens::Screen;
use crate::screens::settings::ThemeChoice;

/// The three screens one sees before the password, opened as their presets open them.
fn login_screen(screen: &str) -> (MutexGuard<'static, ()>, Kit) {
    let (turn, mut kit) = match screen {
        "welcome" => demo_app("welcome", "en"),
        _ => demo_app("wallet-unlock", "en"),
    };
    let want = if screen == "welcome" {
        Screen::Welcome
    } else {
        Screen::Unlock
    };
    wait_for(&mut kit, "the login screen", |kit| {
        kit.state().screen == want
    });
    if screen == "welcome-back" {
        // The demo has three wallets; «Welcome back» is the screen of one. The demo core
        // sends a snapshot only when a command changes it, and these clicks send none.
        if let Some(s) = kit.state_mut().snapshot.as_mut() {
            s.wallets.truncate(1);
        }
        kit.run_steps(2);
        assert!(
            has_text(&kit, "Open the wallet"),
            "«Welcome back» is not shown"
        );
    }
    (turn, kit)
}

fn link<'k>(kit: &'k Kit, name: &'k str) -> egui_kittest::Node<'k> {
    kit.get_by_role_and_label(Role::Link, name)
}

const SCREENS: [&str; 3] = ["welcome", "wallet-unlock", "welcome-back"];

/// A click on the language in the foot opens «Interface language», the dialog of Settings.
#[test]
fn the_language_in_the_login_foot_opens_the_language_dialog() {
    for screen in SCREENS {
        let (_turn, mut kit) = login_screen(screen);
        assert!(
            !kit.state().settings.lang_picker,
            "{screen}: open too early"
        );
        link(&kit, "English").click();
        kit.run_steps(3);
        assert!(kit.state().settings.lang_picker, "{screen}: no dialog");
        assert!(
            has_text(&kit, "Interface language"),
            "{screen}: the dialog is not on screen"
        );
    }
}

/// A click on the theme in the foot switches it, and the foot then names the other one.
#[test]
fn the_theme_in_the_login_foot_switches_the_theme() {
    for screen in SCREENS {
        let (_turn, mut kit) = login_screen(screen);
        let dark = kit
            .query_by_role_and_label(Role::Link, "dark theme")
            .is_some();
        let (now, next, want) = if dark {
            ("dark theme", "light theme", ThemeChoice::Light)
        } else {
            ("light theme", "dark theme", ThemeChoice::Dark)
        };
        link(&kit, now).click();
        kit.run_steps(3);
        assert!(
            kit.state().settings.theme == want,
            "{screen}: the theme stayed"
        );
        assert!(
            kit.query_by_role_and_label(Role::Link, next).is_some(),
            "{screen}: the foot still says «{now}»"
        );
    }
}
