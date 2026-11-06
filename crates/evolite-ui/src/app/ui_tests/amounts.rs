//! FEAT-0007: the «eye» beside a screen's main amount switches the app's one «Hide amounts».

use super::*;
use crate::screens::Screen;

const HIDE: &str = "Hide the amounts";
const SHOW: &str = "Show the amounts";

/// The setting itself, as Settings → Security keeps it.
fn hidden(kit: &Kit) -> bool {
    kit.state().settings.is_on("hide-amounts")
}

/// How many buttons a screen reader calls `name`.
fn buttons(kit: &Kit, name: &str) -> usize {
    kit.query_all_by_role(Role::Button)
        .filter(|n| n.accesskit_node().label().as_deref() == Some(name))
        .count()
}

/// The last button a screen reader calls `name` (the topmost: a menu over the screen).
fn last_button<'k>(kit: &'k Kit, name: &str) -> egui_kittest::Node<'k> {
    kit.query_all_by_role(Role::Button)
        .rfind(|n| n.accesskit_node().label().as_deref() == Some(name))
        .unwrap_or_else(|| panic!("no «{name}»"))
}

/// The open wallet's total, as screens write it with the amounts seen.
fn total(kit: &Kit) -> String {
    let wallet = kit
        .state()
        .snapshot
        .as_ref()
        .and_then(|s| s.wallet.as_ref());
    crate::format::amount(wallet.expect("the demo wallet").balances.total())
}

/// AC-1, AC-D1, AC-D2: every screen with a balance has one eye (the wallet menu and «Earlier
/// addresses» too); one press hides the amounts of the whole app and turns it into «Show the
/// amounts», the next shows them again.
#[test]
fn one_eye_per_screen_switches_the_whole_app() {
    for preset in [
        "overview",
        "shielded",
        "identity",
        "send",
        "receive",
        "wallet-menu",
    ] {
        let (_turn, mut kit) = demo_app(preset, "en");
        // The wallet menu opens over Overview: its own eye and the screen's.
        let eyes = if preset == "wallet-menu" { 2 } else { 1 };
        assert!(!hidden(&kit), "{preset}: shown at first");
        assert_eq!(buttons(&kit, HIDE), eyes, "{preset}: one eye");
        // The last one: the menu's, over the screen.
        last_button(&kit, HIDE).click();
        kit.run_steps(3);
        assert!(hidden(&kit), "{preset}: one press hides");
        assert_eq!(buttons(&kit, SHOW), eyes, "{preset}: the eye says «Show»");
        assert_eq!(buttons(&kit, HIDE), 0, "{preset}");
        last_button(&kit, SHOW).click();
        kit.run_steps(3);
        assert!(!hidden(&kit), "{preset}: the next shows them again");
    }
}

/// AC-1, AC-D3: hidden on Overview, the total and the operations read «••••»; the history
/// elsewhere follows the same setting.
#[test]
fn hidden_amounts_read_dots_everywhere() {
    let (_turn, mut kit) = demo_app("overview", "en");
    let total = total(&kit);
    assert!(has_text(&kit, &total), "the total, seen");
    button(&kit, HIDE).click();
    kit.run_steps(3);
    assert!(!has_text(&kit, &total), "the total, hidden");
    assert!(has_text(&kit, &crate::format::mask(&total)));
    // The setting is the app's: Identity's credits hide too.
    kit.state_mut().screen = Screen::Identity;
    kit.run_steps(3);
    assert_eq!(buttons(&kit, SHOW), 1, "Identity's eye says «Show»");
    assert!(has_text(&kit, "•••• credits\n≈ •••• operations"));
}

/// AC-D3: no reveal on hover — the pointer over the hidden total leaves it «••••».
#[test]
fn the_pointer_does_not_reveal_a_hidden_amount() {
    let (_turn, mut kit) = demo_app("hidden", "en");
    assert!(hidden(&kit), "`--screen hidden` hides");
    let masked = crate::format::mask(&total(&kit));
    kit.get(By::new().role(Role::Label).value(&masked)).hover();
    kit.run_steps(3);
    assert!(has_text(&kit, &masked), "still «••••» under the pointer");
    assert!(!has_text(&kit, &total(&kit)));
}

/// AC-2, AC-D2: the eye and the switch in Settings → Security are one setting, both ways, and it
/// is what the settings file keeps.
#[test]
fn the_eye_and_the_settings_switch_are_one() {
    let (_turn, mut kit) = demo_app("security", "en");
    kit.query_all_by_label("Hide amounts")
        .find(|n| n.accesskit_node().role() != Role::Label)
        .expect("the switch")
        .click();
    kit.run_steps(3);
    assert!(hidden(&kit), "the switch in Settings");
    kit.state_mut().screen = Screen::Overview;
    kit.run_steps(3);
    assert_eq!(buttons(&kit, SHOW), 1, "the eye follows Settings");
    button(&kit, SHOW).click();
    kit.run_steps(3);
    assert!(!hidden(&kit), "the eye switches Settings back");
    button(&kit, HIDE).click();
    kit.run_steps(3);
    let prefs = kit.state().settings.prefs(&Default::default());
    assert_eq!(
        prefs.toggles.get("hide-amounts"),
        Some(&true),
        "written to the file"
    );
    // AC-D9: the switch's line no longer speaks of the pointer.
    kit.state_mut().screen = Screen::Settings;
    kit.run_steps(3);
    assert!(has_text(
        &kit,
        "Balances and history show ••••. The eye button next to an amount switches this too."
    ));
}

/// AC-3, AC-D5: the eye is a Tab stop with a name; Enter and Space switch it.
#[test]
fn tab_reaches_the_eye_and_enter_or_space_switch_it() {
    let (_turn, mut kit) = demo_app("overview", "en");
    tab_to(&mut kit, HIDE);
    enter(&mut kit);
    assert!(hidden(&kit), "Enter hides");
    assert_eq!(
        focused(&kit).as_deref(),
        Some(SHOW),
        "the focus stays, renamed"
    );
    kit.key_press(egui::Key::Space);
    kit.run_steps(2);
    assert!(!hidden(&kit), "Space shows");
}

/// AC-D10: no hotkey — Ctrl+Shift+H changes nothing.
#[test]
fn there_is_no_hotkey() {
    let (_turn, mut kit) = demo_app("overview", "en");
    kit.key_press_modifiers(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, egui::Key::H);
    kit.run_steps(3);
    assert!(!hidden(&kit));
}

/// The open wallet's balances and its total, each as screens write it with the amounts seen
/// (the ones not 0).
fn balances(kit: &Kit) -> Vec<String> {
    let wallet = kit
        .state()
        .snapshot
        .as_ref()
        .and_then(|s| s.wallet.as_ref())
        .expect("the demo wallet");
    let b = &wallet.balances;
    [b.core, b.credits, b.platform_address, b.shielded, b.total()]
        .into_iter()
        .filter(|d| *d > 0)
        .map(crate::format::amount)
        .collect()
}

/// Whether some node's name or words have `text` in them.
fn shows(kit: &Kit, text: &str) -> bool {
    kit.query_all_by(|n| {
        n.label().is_some_and(|l| l.contains(text)) || n.value().is_some_and(|v| v.contains(text))
    })
    .next()
    .is_some()
}

/// Review r1 must 1 and the designer's decisions: with «Hide amounts» on, the balances in the
/// dialogs read «••••» — the pool's «From» and «Left», the identity top-up's «From the Core
/// wallet» and «after · ≈ N operations», «From the Core wallet» of a new identity, «Credits
/// after» of an app's request, «Balance» before deleting; the found wallet has its eye.
#[test]
fn hidden_balances_read_dots_in_the_dialogs() {
    for preset in [
        "shield",
        "shield-core",
        "unshield",
        "top-up",
        "create-identity",
        "sign",
        "delete",
    ] {
        let (_turn, mut kit) = demo_app(preset, "en");
        let seen = balances(&kit);
        // An app's request shows the credits after it, not a balance of the wallet.
        assert!(
            preset == "sign" || seen.iter().any(|b| shows(&kit, b)),
            "{preset}: a balance is seen at first"
        );
        kit.state_mut().settings.flip_hide_amounts();
        kit.run_steps(3);
        for b in &seen {
            assert!(!shows(&kit, b), "{preset}: {b} is hidden");
        }
        assert!(shows(&kit, "••••"), "{preset}: «••••» instead");
    }
    // The top-up's operations after it, as Identity's own.
    let (_turn, mut kit) = demo_app("top-up", "en");
    kit.state_mut().settings.flip_hide_amounts();
    kit.run_steps(3);
    assert!(
        shows(&kit, "•••• DASH · ≈ •••• operations"),
        "top-up: operations hidden"
    );
    drop(kit);
    drop(_turn);
    // An app's request: its cost stays, the credits after it hide.
    let (_turn, mut kit) = demo_app("sign", "en");
    assert!(!shows(&kit, "Credits after: ••••"), "sign: seen at first");
    kit.state_mut().settings.flip_hide_amounts();
    kit.run_steps(3);
    assert!(
        shows(&kit, "Credits after: ••••"),
        "sign: credits after hidden"
    );
    assert!(!shows(&kit, "Cost: ≈ ••••"), "sign: the cost stays");
}

/// «Core has {} now» under «No identity yet» hides the Core balance too.
#[test]
fn the_core_balance_without_an_identity_hides() {
    let (_turn, mut kit) = demo_app("identity", "en");
    let core = {
        let wallet = kit
            .state_mut()
            .snapshot
            .as_mut()
            .and_then(|s| s.wallet.as_mut())
            .expect("the demo wallet");
        wallet.identity = None;
        wallet.unfinished_identity = None;
        crate::format::amount(wallet.balances.core)
    };
    kit.run_steps(3);
    assert!(
        shows(&kit, &format!("Core has {core} now")),
        "seen at first"
    );
    kit.state_mut().settings.flip_hide_amounts();
    kit.run_steps(3);
    assert!(!shows(&kit, &core), "the Core balance is hidden");
    let masked = crate::format::mask(&core);
    assert!(
        shows(&kit, &format!("Core has {masked} now")),
        "«••••» instead"
    );
}

/// The designer's decision for the found wallet: its balances follow the setting, and an eye
/// on the screen shows them — a person knows the wallet by them.
#[test]
fn the_found_wallet_has_its_eye() {
    let (_turn, mut kit) = demo_app("dashpay", "en");
    assert_eq!(buttons(&kit, HIDE), 1, "one eye");
    button(&kit, HIDE).click();
    kit.run_steps(3);
    assert!(hidden(&kit), "the eye hides");
    assert_eq!(buttons(&kit, SHOW), 1, "and says «Show»");
}

/// The amounts of the operation rows on the screen, as a screen reader hears them: a row is
/// «title · amount · detail» (`components::tx_row`). A title may have « · » of its own («Sent ·
/// Exchange · withdrawal»): the amount is the first part in DASH.
fn row_amounts(kit: &Kit) -> Vec<String> {
    kit.query_all_by(|n| n.label().is_some_and(|l| l.split(" · ").count() >= 3))
        .filter_map(|n| {
            let label = n.accesskit_node().label()?.to_string();
            label
                .split(" · ")
                .find(|part| part.ends_with(" DASH"))
                .map(str::to_string)
        })
        .collect()
}

/// Review r2 should 1: with «Hide amounts» on, the operation rows read «••••» — on Overview,
/// in History and under «Receive» — and a screen reader does not say the amount either. (The
/// demo pool has no operations to show; its rows are the same `tx_row`.)
#[test]
fn hidden_amounts_hide_the_operation_rows() {
    for preset in ["overview", "activity", "receive"] {
        let (_turn, mut kit) = demo_app(preset, "en");
        wait_for(&mut kit, "the operations", |kit| {
            !row_amounts(kit).is_empty()
        });
        let seen = row_amounts(&kit);
        assert!(
            seen.iter().any(|a| a.chars().any(|c| c.is_ascii_digit())),
            "{preset}: the amounts are seen at first: {seen:?}"
        );
        kit.state_mut().settings.flip_hide_amounts();
        kit.run_steps(3);
        let now = row_amounts(&kit);
        assert_eq!(now.len(), seen.len(), "{preset}: the same rows");
        for a in &now {
            assert_eq!(a, "•••• DASH", "{preset}: a row's amount is hidden");
        }
    }
}

/// What a screen reader says that starts with `start` («Left: …»): a row's name or a label's
/// words.
fn said(kit: &Kit, start: &str) -> Vec<String> {
    kit.query_all_by(|n| {
        n.label().is_some_and(|l| l.starts_with(start))
            || n.value().is_some_and(|v| v.starts_with(start))
    })
    .filter_map(|n| {
        let n = n.accesskit_node();
        n.label().or(n.value()).map(|s| s.to_string())
    })
    .collect()
}

/// Review r2 should 1: with «Hide amounts» on, the balance of «From» on «Send» reads «••••» in
/// «Check the payment», and so does «Left»; the amount to send and the fee stay seen.
#[test]
fn hidden_amounts_hide_the_balances_to_send_from() {
    let (_turn, mut kit) = demo_app("send-confirm", "en");
    let seen = balances(&kit);
    assert!(
        seen.iter().any(|b| shows(&kit, b)),
        "the balance to send from is seen at first"
    );
    kit.state_mut().settings.flip_hide_amounts();
    kit.run_steps(3);
    for b in &seen {
        assert!(!shows(&kit, b), "{b} is hidden");
    }
    assert!(
        said(&kit, "From: ")
            .iter()
            .any(|s| s.ends_with(" · •••• DASH")),
        "«From» in «Check the payment» reads «••••»: {:?}",
        said(&kit, "From: ")
    );
    let left = said(&kit, "Left: ");
    assert!(
        !left.is_empty() && left.iter().all(|s| s == "Left: •••• DASH"),
        "{left:?}"
    );
    let amount = said(&kit, "Amount: ");
    assert!(
        amount.iter().any(|s| s.chars().any(|c| c.is_ascii_digit())),
        "the amount to send is seen: {amount:?}"
    );
}

/// What «Max» on «Send» puts in the amount field, and what «Left» then says.
fn after_max(hide: bool) -> (String, Vec<String>) {
    let (_turn, mut kit) = demo_app("send", "en");
    if hide {
        kit.state_mut().settings.flip_hide_amounts();
        kit.run_steps(3);
    }
    kit.state_mut().send.amount.clear();
    kit.state_mut().send.max = false;
    kit.run_steps(2);
    button(&kit, "Max").click();
    kit.run_steps(3);
    let amount = kit.state().send.amount.clone();
    assert!(
        kit.query_all_by_role(Role::TextInput)
            .any(|n| n.accesskit_node().value().as_deref() == Some(amount.as_str())),
        "the field shows {amount:?}"
    );
    (amount, said(&kit, "Left: "))
}

/// Review r2 should 2, the designer's decision (FEAT-0007.md, «"Всё" на "Отправить" при
/// скрытых суммах»): «Max» works with the amounts hidden and puts the real amount in the field
/// — the person must see what is sent; «Left» stays «••••».
#[test]
fn max_puts_the_real_amount_in_the_field_with_the_amounts_hidden() {
    let (shown, _) = after_max(false);
    let (amount, left) = after_max(true);
    assert!(
        amount.chars().any(|c| c.is_ascii_digit()) && amount.chars().any(|c| c != '0' && c != '.'),
        "«Max» gives an amount: {amount:?}"
    );
    assert_eq!(amount, shown, "the same amount as with the amounts seen");
    assert_eq!(left, ["Left: •••• DASH"], "«Left» is hidden");
}
