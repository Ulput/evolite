//! BUG-74: what screens show while things load — no QR without its address, no red «At least…»
//! while identities are looked for, no «0» beside «Loading NFTs…», and the first Tab on the
//! screen, not in the menu.

use super::*;
use crate::screens::Screen;
use evolite_api::{IdentitySearch, NftsStatus, Wallet};

/// Changes the wallet the app shows and draws, again for a few frames: a snapshot of the demo
/// core in between would put its own back.
fn with_wallet(kit: &mut Kit, change: impl Fn(&mut Wallet)) {
    for _ in 0..4 {
        if let Some(w) = kit
            .state_mut()
            .snapshot
            .as_mut()
            .and_then(|s| s.wallet.as_mut())
        {
            change(w);
        }
        kit.run_steps(1);
    }
}

/// Whether the button `name` is drawn and can be pressed.
fn enabled(kit: &Kit, name: &str) -> bool {
    kit.query_all_by_role_and_label(Role::Button, name)
        .any(|b| !b.accesskit_node().is_disabled())
}

const PREPARING: &str =
    "Preparing the address from the wallet keys — no network needed, usually a second.";

/// №1: no address yet — no QR, nothing to copy, and why; with the address all of it.
#[test]
fn receive_shows_no_qr_and_copies_nothing_without_the_address() {
    let (_turn, mut kit) = demo_app("receive", "en");
    assert!(enabled(&kit, "Copy address") && enabled(&kit, "dash: link"));
    assert!(!has_text(&kit, PREPARING));
    let address = kit
        .state()
        .snapshot
        .as_ref()
        .and_then(|s| s.wallet.as_ref())
        .map(|w| w.core_address.clone())
        .expect("a wallet");
    with_wallet(&mut kit, |w| w.core_address.clear());
    assert!(has_text(&kit, PREPARING), "why there is no address");
    assert!(
        !enabled(&kit, "Copy address") && !enabled(&kit, "dash: link"),
        "nothing to copy"
    );
    with_wallet(&mut kit, |w| w.core_address = address.clone());
    assert!(has_text(&kit, &address) && enabled(&kit, "Copy address"));
    assert!(!has_text(&kit, PREPARING));
}

/// Whether some words on screen start with `part`.
fn says_start(kit: &Kit, part: &str) -> bool {
    kit.query_all_by(|n| {
        n.label().is_some_and(|l| l.starts_with(part))
            || n.value().is_some_and(|v| v.starts_with(part))
    })
    .next()
    .is_some()
}

/// №2, CS-4.1: while identities are looked for, no «At least ≈ … is needed» and no buttons — one
/// may yet be found; once the search is done it shows, in red only for a Core the sync has
/// counted. A failed search is not done either: no amount line.
#[test]
fn identity_says_nothing_of_money_while_it_searches() {
    let (_turn, mut kit) = demo_app("identity", "en");
    let none_yet = |w: &mut Wallet, search: IdentitySearch| {
        w.identity = None;
        w.identities.clear();
        w.identity_search = search;
        w.balances.core = 0;
    };
    with_wallet(&mut kit, |w| none_yet(w, IdentitySearch::Searching));
    assert!(
        says_start(&kit, "Looking for this phrase's identity on Platform"),
        "the search is said"
    );
    assert!(says_start(
        &kit,
        "We check the answer against masternode lists"
    ));
    assert!(
        !says_start(&kit, "At least ≈ "),
        "no money line while it searches"
    );
    for none in ["Create an identity", "Search again"] {
        assert!(
            kit.query_by_role_and_label(Role::Button, none).is_none(),
            "«{none}» while it searches"
        );
    }
    with_wallet(&mut kit, |w| {
        none_yet(w, IdentitySearch::Failed("no answer".into()))
    });
    assert!(
        !says_start(&kit, "At least ≈ "),
        "no money line after a failed search"
    );
    with_wallet(&mut kit, |w| none_yet(w, IdentitySearch::Done));
    assert!(says_start(&kit, "At least ≈ "), "after the search");
    assert!(!says_start(
        &kit,
        "Looking for this phrase's identity on Platform"
    ));
    button(&kit, "Create an identity");
    button(&kit, "Search again");
}

/// №3: no count beside «Collection» while the list loads, nor for none; a count for NFTs there.
#[test]
fn the_collection_counts_only_nfts_there_are() {
    let (_turn, mut kit) = demo_app("collection", "en");
    let n = kit
        .state()
        .snapshot
        .as_ref()
        .and_then(|s| s.wallet.as_ref())
        .map_or(0, |w| w.nfts.len());
    assert!(n > 0, "the demo has NFTs");
    assert!(has_text(&kit, &n.to_string()), "the count beside the title");
    with_wallet(&mut kit, |w| {
        w.nfts.clear();
        w.nfts_status = NftsStatus::Loading;
    });
    assert!(has_text(&kit, "Loading NFTs from Dash Platform…"));
    assert!(!has_text(&kit, "0"), "no «0» while it loads");
    with_wallet(&mut kit, |w| {
        w.nfts.clear();
        w.nfts_status = NftsStatus::Ready;
    });
    assert!(!has_text(&kit, "0"), "no «0» for none");
}

/// Where the focused widget starts: the menu is the left 76 px at 1024 wide (the rail).
fn focused_left(kit: &Kit) -> Option<f32> {
    kit.query_by(|n| n.is_focused()).map(|n| n.rect().left())
}

/// №4: the first Tab goes to the screen, not to the menu; the menu comes after the screen's
/// last widget, and Shift+Tab from nothing goes back as before.
#[test]
fn the_first_tab_goes_to_the_screen() {
    for (preset, screen) in [
        ("send", Screen::Send),
        ("overview", Screen::Overview),
        ("receive", Screen::Receive),
    ] {
        let (_turn, mut kit) = demo_app(preset, "en");
        assert_eq!(kit.state().screen, screen);
        assert!(
            focused(&kit).is_none(),
            "{preset}: nothing focused at first"
        );
        tab(&mut kit, false);
        let first = focused(&kit).unwrap_or_else(|| panic!("{preset}: Tab focused nothing"));
        let rect = kit
            .query_by(|n| n.is_focused())
            .map(|n| n.rect())
            .expect("the focused node");
        assert!(
            rect.left() > 76.0,
            "{preset}: the first Tab on «{first}» {rect:?}, in the menu"
        );
        // On round: the menu after the screen.
        let mut menu = false;
        for _ in 0..80 {
            tab(&mut kit, false);
            if focused_left(&kit).is_some_and(|x| x < 76.0) {
                menu = true;
                break;
            }
        }
        assert!(menu, "{preset}: Tab never came to the menu");
    }
}

/// №4 with the full menu (1280 wide): the first Tab goes to the screen too.
#[test]
fn with_the_full_menu_the_first_tab_goes_to_the_screen() {
    let (_turn, mut kit) = demo_app("overview", "en");
    kit.set_size(egui::vec2(1280.0, 800.0));
    kit.run_steps(2);
    tab(&mut kit, false);
    let first = focused(&kit).expect("Tab focused something");
    let rect = kit
        .query_by(|n| n.is_focused())
        .map(|n| n.rect())
        .expect("the focused node");
    assert!(rect.left() > 248.0, "the first Tab on «{first}» {rect:?}");
}
