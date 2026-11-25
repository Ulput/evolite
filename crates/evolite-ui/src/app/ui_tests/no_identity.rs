//! BUG-262: the screens that need an identity, in a wallet with none — their empty state, never a
//! button that does nothing (A1, A5), «Looking for…» while the phrase's identity is looked for
//! (A2), «Continue creating» for a creation cut short (A3), and no empty state while any identity
//! is there (A4).

use super::*;
use crate::screens::Screen;

/// Each screen, its title and its line (the card's table).
const SCREENS: [(&str, &str, &str); 6] = [
    (
        "no-identity-contacts",
        "Contacts appear once you have an identity",
        "DashPay contacts and payments by @name work on behalf of an identity.",
    ),
    (
        "no-identity-tokens",
        "Tokens appear once you have an identity",
        "Dash Platform tokens are held by an identity.",
    ),
    (
        "no-identity-collection",
        "Your collection appears once you have an identity",
        "NFTs are held by an identity.",
    ),
    (
        "no-identity-workshop",
        "The Workshop works once you have an identity",
        "Collections and tokens are issued on behalf of an identity and paid with its credits.",
    ),
    (
        "no-identity-apps",
        "Apps appear once you have an identity",
        "You sign in to Dash apps on behalf of an identity.",
    ),
    (
        "no-identity-identity",
        "You have no identity yet",
        "An identity is your record on the Dash network: a name, keys and credits.",
    ),
];

/// The buttons drawn off.
fn off(kit: &Kit) -> Vec<String> {
    kit.query_all_by_role(Role::Button)
        .filter(|b| b.accesskit_node().is_disabled())
        .map(|b| b.accesskit_node().label().unwrap_or_default())
        .collect()
}

/// A1, A5, R-1: on each screen the title — read as the screen's heading — and the line, «Create
/// an identity» on and leading to the creation, and no button drawn off anywhere.
#[test]
fn each_screen_without_an_identity_shows_its_empty_state() {
    for (preset, title, line) in SCREENS {
        let (_turn, mut kit) = demo_app(preset, "en");
        settle(&mut kit);
        assert!(has_text(&kit, title), "{preset}: «{title}»");
        assert!(has_words(&kit, line), "{preset}: «{line}»");
        assert!(
            kit.query_all_by_role(Role::Heading).any(|n| {
                let n = n.accesskit_node();
                n.label().as_deref() == Some(title) || n.value().as_deref() == Some(title)
            }),
            "{preset}: the title is a heading"
        );
        assert!(
            off(&kit).is_empty(),
            "{preset}: buttons off: {:?}",
            off(&kit)
        );
        let create = button(&kit, "Create an identity");
        assert!(!create.accesskit_node().is_disabled());
        create.click();
        settle(&mut kit);
        assert_eq!(
            kit.state().screen,
            Screen::CreateIdentity,
            "{preset}: to the creation (FEAT-0048)"
        );
    }
}

/// A2 (R-2): while the phrase's identity is still looked for, «Looking for…» and no button — a
/// second identity would be made.
#[test]
fn while_the_identity_is_looked_for_nothing_is_offered() {
    let (_turn, mut kit) = demo_app("no-identity-contacts", "en");
    settle(&mut kit);
    show_wallet_edited(
        &mut kit,
        3,
        "the search running",
        |w| w.identity_search = evolite_api::IdentitySearch::Searching,
        |_| true,
    );
    assert!(has_words(
        &kit,
        "Looking for this phrase's identity on Platform"
    ));
    assert!(
        kit.query_all_by_role_and_label(Role::Button, "Create an identity")
            .next()
            .is_none(),
        "no «Create an identity»"
    );
}

/// r1 Should 1 (§8d, the third state): the search failed — on each screen but Identity (which
/// says it above its own) why and «Search again», never «Create an identity»; no button off.
#[test]
fn a_failed_search_offers_to_search_again_not_to_create() {
    for (preset, title, _) in SCREENS
        .into_iter()
        .filter(|(p, ..)| *p != "no-identity-identity")
    {
        let (_turn, mut kit) = demo_app(preset, "en");
        settle(&mut kit);
        show_wallet_edited(
            &mut kit,
            3,
            "the search failed",
            |w| w.identity_search = evolite_api::IdentitySearch::Failed("no answer".into()),
            |_| true,
        );
        assert!(has_text(&kit, title), "{preset}: its title");
        assert!(
            has_words(
                &kit,
                "Could not ask Platform about this phrase's identities (no answer)"
            ),
            "{preset}: why"
        );
        assert!(
            kit.query_all_by_role_and_label(Role::Button, "Create an identity")
                .next()
                .is_none(),
            "{preset}: no «Create an identity»"
        );
        assert!(
            !button(&kit, "Search again").accesskit_node().is_disabled(),
            "{preset}: «Search again» on"
        );
        assert!(
            off(&kit).is_empty(),
            "{preset}: buttons off: {:?}",
            off(&kit)
        );
    }
}

/// A3 (R-3): a creation cut short — «Continue creating», not a new one.
#[test]
fn a_creation_cut_short_is_continued_not_started_anew() {
    let (_turn, mut kit) = demo_app("no-identity-tokens", "en");
    settle(&mut kit);
    show_wallet_edited(
        &mut kit,
        3,
        "a creation cut short",
        |w| {
            w.unfinished_pool_identity = Some(evolite_api::UnfinishedPoolIdentity {
                exit: 3_000_000,
                name: None,
                sent_at: chrono::Local::now().timestamp() - 60,
                refusal_address: "tdashevo1q".into(),
                refused: None,
                not_delivered: false,
            });
        },
        |_| true,
    );
    assert!(has_text(&kit, "Continue creating"));
    assert!(
        kit.query_all_by_role_and_label(Role::Button, "Create an identity")
            .next()
            .is_none()
    );
}

/// A4 (R-4): identities there but none chosen (the chosen one deleted or not found) — no empty
/// state: the chosen one is always there while any is.
#[test]
fn no_empty_state_while_any_identity_is_there() {
    let (_turn, mut kit) = demo_app("contacts", "en");
    settle(&mut kit);
    let identities = kit
        .state()
        .snapshot
        .as_ref()
        .and_then(|s| s.wallet.as_ref())
        .map_or(0, |w| w.identities.len());
    assert!(identities > 0, "the demo has identities");
    show_wallet_edited(&mut kit, 3, "none chosen", |w| w.identity = None, |_| true);
    assert!(!has_text(&kit, "Contacts appear once you have an identity"));
}
