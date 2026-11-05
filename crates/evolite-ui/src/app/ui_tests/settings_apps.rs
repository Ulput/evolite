//! Branch A: Settings → Network ("Own node") and Apps.

use super::*;

/// BUG-39 №11: the «Core node address» field opens right under «Own node», not under the
/// «Proxy» row after it.
#[test]
fn the_own_node_field_is_right_under_its_switch() {
    let (_turn, mut kit) = demo_app("own-node", "en");
    kit.run_steps(3);
    let own = words(&kit, "Own node").rect();
    let proxy = words(&kit, "Proxy (SOCKS5)").rect();
    let field = words(&kit, "Core node address").rect();
    assert!(proxy.top() < own.top(), "«Proxy» comes before «Own node»");
    assert!(
        field.top() > own.bottom() && field.top() - own.bottom() < 80.0,
        "the field ({field:?}) follows «Own node» ({own:?})"
    );
}

/// Designer review №17: with no app connected, the empty card says what to do and has the
/// button for it.
#[test]
fn no_connected_apps_shows_the_next_step() {
    let (_turn, mut kit) = demo_app("apps", "en");
    if let Some(w) = kit
        .state_mut()
        .snapshot
        .as_mut()
        .and_then(|s| s.wallet.as_mut())
    {
        w.apps.clear();
    }
    kit.run_steps(3);
    assert!(has_text(&kit, "No connected apps"));
    assert!(has_text(
        &kit,
        "To connect one, paste the link the app shows."
    ));
    let pastes: Vec<egui::Rect> = kit
        .query_all_by_role_and_label(Role::Button, "Paste a link")
        .map(|n| n.rect())
        .collect();
    assert_eq!(pastes.len(), 2, "the header's and the empty card's");
    kit.query_all_by_role_and_label(Role::Button, "Paste a link")
        .last()
        .expect("the empty card's button")
        .click();
    kit.run_steps(3);
    assert!(kit.state().requests.pasting, "the paste dialog opens");
}
