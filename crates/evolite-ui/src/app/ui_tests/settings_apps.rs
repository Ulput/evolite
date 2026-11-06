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

/// BUG-30: after «Allow sign-in» / «Sign» / «The code matches» the core has taken the request,
/// so once it answers the dialog offers neither a second yes nor «Decline» — only «Close», and
/// closing it sends no "no". (The demo core turns every yes down: there is no wallet behind it.)
#[test]
fn after_a_yes_only_close_is_left() {
    for (preset, yes, no) in [
        ("connect", "Allow sign-in", "Decline"),
        ("sign", "Sign", "Decline"),
        ("exchange", "The code matches", "The code does not match"),
    ] {
        let (_turn, mut kit) = demo_app(preset, "en");
        wait_for(&mut kit, "the request dialog", |kit| {
            kit.query_by_role_and_label(Role::Button, yes).is_some()
        });
        button(&kit, yes).click();
        kit.run_steps(2);
        wait_for(&mut kit, "the core's answer to the yes", |kit| {
            kit.state().requests.result.is_some()
        });
        kit.run_steps(2);
        assert!(
            kit.query_by_role_and_label(Role::Button, yes).is_none(),
            "{preset}: no second «{yes}» after the answer"
        );
        assert!(
            kit.query_by_role_and_label(Role::Button, no).is_none(),
            "{preset}: no «{no}» after the yes"
        );
        // The «Close» under the answer; the dialog's × is «Close» too (BUG-41), first in the tree.
        kit.query_all_by_role_and_label(Role::Button, "Close")
            .last()
            .expect("«Close»")
            .click();
        kit.run_steps(2);
        assert!(kit.state().requests.open.is_none(), "{preset}: closed");
    }
}

/// BUG-28: «Source code · GitHub ›» in About opens the repository in the browser, from its
/// words and from «GitHub ›» alike (in tests the link is only noted).
#[test]
fn the_source_code_row_opens_the_repository() {
    let (_turn, mut kit) = demo_app("about-end", "en");
    kit.run_steps(3);
    let before = crate::links::opened().len();
    words(&kit, "Source code").click();
    kit.run_steps(2);
    words(&kit, "GitHub").click();
    kit.run_steps(2);
    assert_eq!(
        crate::links::opened()[before..],
        [
            "https://github.com/Ulput/evolite",
            "https://github.com/Ulput/evolite"
        ],
        "each click asks the browser for the repository"
    );
}

/// BUG-29 (the designer, 2026-11-06): «Sign in as» is a plain row of text, not a list that does
/// not open — the core signs in as the identity of «Work as» — and with more than one identity
/// a muted line says where to change it.
///
/// Failed on dev 50c5bd4: the row was a ComboBox («Sign in as: @…») and said nothing more.
#[test]
fn sign_in_as_is_text_that_says_where_to_change_it() {
    const HINT: &str =
        "Sign-in uses the identity from “Work as”. You can change it on the Identity screen.";
    let (_turn, mut kit) = demo_app("connect", "en");
    wait_for(&mut kit, "the sign-in request", |kit| {
        kit.query_by_role_and_label(Role::Button, "Allow sign-in")
            .is_some()
    });
    let says = |n: &egui_kittest::Node<'_>| {
        let a = n.accesskit_node();
        [a.label(), a.value()]
            .into_iter()
            .flatten()
            .any(|t| t.starts_with("Sign in as:"))
    };
    let shown = match kit.state().requests.open.as_ref().map(|r| &r.kind) {
        Some(evolite_api::AppRequestKind::Connect { identity, .. }) => identity.clone(),
        _ => panic!("a sign-in request"),
    };
    assert!(
        kit.query_all_by_role(Role::ComboBox)
            .all(|n| !says(&n) && n.accesskit_node().label().as_deref() != Some(&shown)),
        "«Sign in as» is not a list"
    );
    assert!(
        kit.query_all_by_role(Role::Label).any(|n| says(&n)),
        "«Sign in as: …» is read as text"
    );
    let identities = |kit: &Kit| {
        kit.state()
            .snapshot
            .as_ref()
            .and_then(|s| s.wallet.as_ref())
            .map_or(0, |w| w.identities.len())
    };
    assert!(identities(&kit) > 1, "the demo wallet has two identities");
    assert!(
        has_text(&kit, HINT),
        "with two identities, where to change it"
    );
    // One identity: nothing to change, nothing said (again for a few frames: a snapshot of the
    // demo core in between brings the second one back).
    for _ in 0..3 {
        if let Some(w) = kit
            .state_mut()
            .snapshot
            .as_mut()
            .and_then(|s| s.wallet.as_mut())
        {
            w.identities.truncate(1);
        }
        kit.step();
        if identities(&kit) == 1 {
            break;
        }
    }
    assert_eq!(identities(&kit), 1);
    assert!(!has_text(&kit, HINT), "with one identity, no line about it");
}
