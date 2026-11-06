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

/// Designer review №8 (branch D2): one order in every dialog — the secondary button («Cancel»
/// when the dialog acts, «Close» when it only shows) on the left, the actions on its right, in
/// one row as high as a button that settles and stays put.
#[test]
fn every_dialog_puts_the_secondary_button_on_the_left() {
    for (preset, secondary) in [
        ("send-confirm", "Cancel"),
        ("report", "Cancel"),
        ("report-letter", "Back"),
        ("top-up", "Cancel"),
        ("add-name", "Cancel"),
        ("edit-profile", "Cancel"),
        ("add-contact", "Cancel"),
        ("shield", "Cancel"),
        ("unshield", "Cancel"),
        ("pool-address", "Close"),
        ("wallet-switch", "Cancel"),
        ("resync", "Cancel"),
        ("wallet-rename", "Cancel"),
        ("change-password", "Cancel"),
        ("phrase", "Cancel"),
        ("delete", "Cancel"),
        ("connect", "Decline"),
    ] {
        let (_turn, mut kit) = demo_app(preset, "en");
        wait_for(&mut kit, preset, |kit| {
            kit.query_all_by_role_and_label(Role::Button, secondary)
                .next()
                .is_some()
        });
        // The dialog's own: drawn over the page, so last (a page may have its «Cancel» too).
        let last = |kit: &Kit| {
            kit.query_all_by_role_and_label(Role::Button, secondary)
                .last()
                .map(|b| b.rect())
                .expect("the secondary button")
        };
        // The row settles (the demo core's answers may still lengthen the body) and then stays:
        // a row that takes all the height left would grow the dialog every frame.
        let mut left = last(&kit);
        let mut same = 0;
        for _ in 0..40 {
            kit.step();
            let now = last(&kit);
            same = if now == left { same + 1 } else { 0 };
            left = now;
            if same == 4 {
                break;
            }
        }
        assert_eq!(same, 4, "{preset}: the row of buttons does not settle");
        assert_eq!(left.height(), 41.0, "{preset}: «{secondary}» is a button");
        let row: Vec<_> = kit
            .query_all_by_role(Role::Button)
            .map(|b| b.rect())
            .filter(|r| (r.center().y - left.center().y).abs() < 1.0 && *r != left)
            .collect();
        assert!(!row.is_empty(), "{preset}: an action beside «{secondary}»");
        for r in &row {
            assert!(
                r.left() >= left.right(),
                "{preset}: an action at {r:?} left of «{secondary}» at {left:?}"
            );
        }
    }
}

/// The designer, 2026-11-06 (b): Tab goes along a dialog's row of buttons as it is read — from
/// the secondary to the action next to it, on its right, and on its left in Arabic, where the
/// row is mirrored — not from the primary back to the secondary (WCAG 2.4.3).
///
/// Failed on dev 50c5bd4: the row was drawn right to left, so Tab came to the primary first
/// and from «Cancel» left the row.
#[test]
fn tab_goes_along_the_row_of_buttons_as_it_is_read() {
    let mut checked = Vec::new();
    for (lang, preset, secondary) in [
        ("en", "report-letter", "Back"),
        ("en", "pool-address", "Close"),
        ("en", "resync", "Cancel"),
        ("en", "connect", "Decline"),
        ("en", "add-contact", "Cancel"),
        ("en", "top-up", "Cancel"),
        ("en", "shield", "Cancel"),
        ("en", "wallet-rename", "Cancel"),
        ("en", "report", "Cancel"),
        ("ar", "pool-address", "Close"),
        ("ar", "resync", "Cancel"),
        ("ar", "report-letter", "Back"),
    ] {
        let (_turn, mut kit) = demo_app(preset, lang);
        let secondary = crate::i18n::spoken(secondary);
        let what = format!("{preset} {lang}: «{secondary}»");
        wait_for(&mut kit, &what, |kit| {
            kit.query_all_by_role_and_label(Role::Button, &secondary)
                .next()
                .is_some()
        });
        kit.run_steps(4);
        // The dialog's own, drawn over the page: the last one.
        let at = kit
            .query_all_by_role_and_label(Role::Button, &secondary)
            .last()
            .map(|b| b.rect())
            .expect("the secondary button");
        // The action read next: the nearest enabled button of the row on the reading side.
        let rtl = lang == "ar";
        let next = kit
            .query_all_by_role(Role::Button)
            .filter(|b| !b.accesskit_node().is_disabled())
            .map(|b| b.rect())
            .filter(|r| (r.center().y - at.center().y).abs() < 1.0 && *r != at)
            .filter(|r| {
                if rtl {
                    r.right() <= at.left()
                } else {
                    r.left() >= at.right()
                }
            })
            .min_by(|a, b| {
                let d = |r: &egui::Rect| (r.center().x - at.center().x).abs();
                d(a).total_cmp(&d(b))
            });
        let Some(next) = next else {
            continue;
        };
        let focus = |kit: &Kit| kit.query_by(|n| n.is_focused()).map(|n| n.rect());
        for _ in 0..80 {
            if focus(&kit) == Some(at) {
                break;
            }
            tab(&mut kit, false);
        }
        assert_eq!(focus(&kit), Some(at), "{what}: Tab never came to it");
        tab(&mut kit, false);
        assert_eq!(
            focus(&kit),
            Some(next),
            "{what}: Tab went to «{}», not to the action read after it",
            focused(&kit).unwrap_or_default()
        );
        // And Shift+Tab goes back.
        tab(&mut kit, true);
        assert_eq!(focus(&kit), Some(at), "{what}: Shift+Tab did not come back");
        checked.push(format!("{preset} {lang}"));
    }
    assert!(
        checked.len() >= 6 && checked.iter().any(|c| c.ends_with(" ar")),
        "too few rows with an enabled action to check: {checked:?}"
    );
}

/// The designer, 2026-11-06 (b): with the row in reading order, Enter in a field still answers
/// with the primary button, not with «Cancel» drawn before it.
#[test]
fn enter_in_a_field_still_takes_the_primary_button() {
    let (_turn, mut kit) = rename_field("Savings");
    enter(&mut kit);
    let rename = kit.state().settings.rename.as_ref();
    assert!(
        rename.is_none_or(|f| f.waiting),
        "Enter did not save the name"
    );
}
