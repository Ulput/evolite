//! BUG-97: a name is two paid steps, and Platform wants the domain's peak out of what remains
//! after the preorder. The form starts nothing the credits cannot finish.

use super::*;
use crate::screens::Screen;
use evolite_api::{UnfinishedIdentity, Wallet};

const NEEDED: i64 = 165_000;

fn shows(kit: &Kit, text: &str) -> bool {
    kit.query_all_by(|n| {
        n.label().is_some_and(|l| l.contains(text)) || n.value().is_some_and(|v| v.contains(text))
    })
    .next()
    .is_some()
}

/// Whether the button called `name` is drawn, and on.
fn on(kit: &Kit, name: &str) -> Option<bool> {
    kit.query_all_by_role(Role::Button)
        .find(|b| b.accesskit_node().label().as_deref() == Some(name))
        .map(|b| !b.accesskit_node().is_disabled())
}

/// Puts `change` on the wallet the app shows until `seen` holds on the same frame: the demo
/// core's own snapshots may come first and put theirs back.
fn shown_with(
    kit: &mut Kit,
    change: impl Fn(&mut Wallet),
    seen: impl Fn(&Kit) -> bool,
    what: &str,
) {
    for _ in 0..80 {
        if let Some(wallet) = kit
            .state_mut()
            .snapshot
            .as_mut()
            .and_then(|s| s.wallet.as_mut())
        {
            change(wallet);
        }
        kit.run_steps(1);
        if seen(kit) {
            return;
        }
    }
    panic!("not seen: {what}");
}

/// The add-name form, its name checked as free, on an identity that holds `credits`.
fn form_with(credits: i64, read: bool, first: bool) -> (std::sync::MutexGuard<'static, ()>, Kit) {
    let (turn, mut kit) = demo_app("add-name", "en");
    wait_for(&mut kit, "the name checked", |kit| {
        shows(kit, "is free · a regular name")
    });
    let change = move |w: &mut Wallet| {
        w.balances.credits = credits;
        w.balances.credits_read = read;
        w.fees.name_regular = 57_979;
        if first && let Some(identity) = &mut w.identity {
            identity.names.clear();
        }
    };
    let price = if first { 78_000 } else { 57_979 };
    shown_with(
        &mut kit,
        change,
        move |kit| {
            let w = kit
                .state()
                .snapshot
                .as_ref()
                .unwrap()
                .wallet
                .as_ref()
                .unwrap();
            w.balances.credits == credits
                && w.fees.name_regular == 57_979
                && shows(kit, &format!("≈ {}", crate::format::amount(price)))
        },
        "the form's fees and credits",
    );
    (turn, kit)
}

/// The two refused attempts of pass 12, and the one that went through, read off the form.
#[test]
fn a_name_the_credits_cannot_finish_is_not_started() {
    let (_turn, mut kit) = form_with(102_105, true, true);
    assert_eq!(on(&kit, "Register"), Some(false), "«Register» off");
    assert!(
        shows(&kit, &format!("≈ {} needed", crate::format::amount(NEEDED))),
        "says how much is needed"
    );
    assert!(
        shows(&kit, "Platform wants this much"),
        "says why it is more than the price"
    );
    assert_eq!(on(&kit, "Top up ›"), Some(true), "offers the top-up");
    kit.get_by_role_and_label(Role::Button, "Top up ›").click();
    kit.run_steps(3);
    assert!(kit.state().identity.adding_name.is_none(), "the name waits");
    assert!(kit.state().identity.top_up.is_some(), "the top-up opens");
}

#[test]
fn credits_that_can_finish_let_the_name_start() {
    let (_turn, kit) = form_with(180_406, true, true);
    assert_eq!(on(&kit, "Register"), Some(true), "«Register» on");
    assert!(!shows(&kit, "Not enough credits for the name"));
    assert_eq!(on(&kit, "Top up ›"), None, "nothing to top up");
}

#[test]
fn the_edge_of_what_a_name_needs() {
    let (_turn, kit) = form_with(NEEDED, true, true);
    assert_eq!(on(&kit, "Register"), Some(true), "exactly enough is enough");
    drop(kit);
    drop(_turn);
    let (_turn, kit) = form_with(NEEDED - 1, true, true);
    assert_eq!(on(&kit, "Register"), Some(false), "one duff short is short");
}

/// BUG-91: credits not read yet are not a 0 and not plenty: nothing can start, and nothing
/// says how many it has.
#[test]
fn credits_not_read_yet_start_nothing_and_say_so() {
    let (_turn, kit) = form_with(0, false, true);
    assert_eq!(on(&kit, "Register"), Some(false), "«Register» off");
    assert!(shows(&kit, "credits are not read yet"), "says so");
    assert!(
        !shows(&kit, "Not enough credits for the name"),
        "not a shortage"
    );
    assert_eq!(on(&kit, "Top up ›"), None, "nothing to top up yet");
}

/// The form promised 57 979 for a first name; a first name measured 77 200 (pass 8).
#[test]
fn a_first_name_is_priced_as_a_first_name() {
    let (_turn, kit) = form_with(180_406, true, true);
    assert!(shows(&kit, &format!("≈ {}", crate::format::amount(78_000))));
    assert!(!shows(
        &kit,
        &format!("≈ {}", crate::format::amount(57_979))
    ));
}

#[test]
fn a_later_name_is_priced_as_the_last_one_cost() {
    let (_turn, kit) = form_with(180_406, true, false);
    assert!(shows(&kit, &format!("≈ {}", crate::format::amount(57_979))));
}

/// The other way to a name: «Continue» after the domain failed. It is the shown identity's
/// credits that have to cover the name again.
#[test]
fn continue_is_off_while_the_made_identity_cannot_pay_for_the_name() {
    let (_turn, mut kit) = demo_app("create-identity", "en");
    wait_for(&mut kit, "the wallet", |kit| kit.state().snapshot.is_some());
    kit.state_mut().screen = Screen::CreateIdentity;
    let unfinished = |w: &mut Wallet, credits: i64| {
        let id = w.identity.as_ref().map(|i| i.id.clone());
        w.balances.credits = credits;
        w.balances.credits_read = true;
        w.fees.name_regular = 57_979;
        w.unfinished_identity = Some(UnfinishedIdentity {
            locked: None,
            name: Some("satoshi-2026".into()),
            identity: id,
        });
    };
    shown_with(
        &mut kit,
        |w| unfinished(w, 102_105),
        |kit| on(kit, "Continue") == Some(false),
        "«Continue» off",
    );
    assert!(shows(
        &kit,
        &format!("≈ {} needed", crate::format::amount(NEEDED))
    ));
    assert_eq!(
        on(&kit, "Top up"),
        Some(true),
        "the top-up of that identity"
    );
    shown_with(
        &mut kit,
        |w| unfinished(w, 180_406),
        |kit| on(kit, "Continue") == Some(true),
        "«Continue» on",
    );
}
