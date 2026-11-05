//! Identity, the private pool and the collection (branch I: BUG-46 and the designer's review
//! 2026-11-05 №7, 14, 15, 16, 27): how their cards stand in the 1024 px window.
//!
//! Cards are frames, not in the tree: a card's edge is a label's edge less the card's margin
//! (24 + the 1 px line, `components::card`) and what stands before the label in it.

use super::*;
use crate::i18n::{Lang, spoken, t_in};
use evolite_api::{IdentityChoice, Wallet};

const WINDOW: f32 = 1024.0;
/// «Platform credits» and «In the pool»: margin 24 + line 1 + icon 18 + gap 8.
const TITLE_AFTER_ICON: f32 = 51.0;
/// A list card's title («DPNS names», «Keys»): margin 20 + line 1.
const LIST_TITLE: f32 = 21.0;
/// «How it works»: margin 24 + line 1.
const PLAIN_TITLE: f32 = 25.0;
/// The name in the profile card: margin 24 + line 1 + avatar 72 + gap 16.
const NAME_IN_PROFILE: f32 = 113.0;

/// Changes the wallet the app shows, and draws, a few times over: a snapshot of the demo core
/// in between would undo it.
fn with_wallet(kit: &mut Kit, change: impl Fn(&mut Wallet)) {
    for _ in 0..3 {
        if let Some(wallet) = kit
            .state_mut()
            .snapshot
            .as_mut()
            .and_then(|s| s.wallet.as_mut())
        {
            change(wallet);
        }
        kit.run_steps(1);
    }
}

/// The shown identity's name, as the profile card writes it.
fn shown_name(kit: &Kit) -> String {
    let wallet = kit
        .state()
        .snapshot
        .as_ref()
        .and_then(|s| s.wallet.as_ref());
    let name = wallet
        .and_then(|w| w.identity.as_ref())
        .and_then(|i| i.name());
    format!("@{}", name.expect("the demo identity has a name"))
}

/// The name in the profile card: the topmost of its labels (the «DPNS names» card says it too).
fn profile_name(kit: &Kit) -> egui::Rect {
    let name = shown_name(kit);
    kit.query_all(By::new().role(Role::Label).value(&name))
        .map(|n| n.rect())
        .min_by(|a, b| a.top().total_cmp(&b.top()))
        .expect("the name on screen")
}

fn close(a: f32, b: f32, what: &str) {
    assert!((a - b).abs() <= 1.0, "{what}: {a} and {b}");
}

/// BUG-46, its steps: «Work as» an identity with a 24-character name. The name wraps or ends in
/// «…» inside its card; the «Platform credits» card and its «Transfer» stay in the window.
#[test]
fn a_long_name_keeps_the_credits_in_the_window() {
    let (_turn, mut kit) = demo_app("identity", "ru");
    with_wallet(&mut kit, |w| {
        if let Some(id) = &mut w.identity {
            for n in &mut id.names {
                n.label = "evolite-probe-115567219".into();
            }
        }
    });
    let credits = words(&kit, &t_in(Lang::Ru, "Platform credits")).rect();
    let transfer = button(&kit, &spoken("Transfer")).rect();
    eprintln!("«Кредиты Platform» {credits:?}, «Перевести» {transfer:?}");
    assert!(
        credits.right() <= WINDOW && transfer.right() <= WINDOW,
        "the credits card stands past the window: «Перевести» ends at {}",
        transfer.right()
    );
}

/// №7: one grid on Identity — the two cards of each row are halves with a gap of 20, and the
/// second column begins where it does in the row below.
#[test]
fn identity_cards_stand_on_one_grid() {
    let (_turn, kit) = demo_app("identity", "en");
    let profile = profile_name(&kit).left() - NAME_IN_PROFILE;
    let credits = words(&kit, "Platform credits").rect().left() - TITLE_AFTER_ICON;
    let names = words(&kit, "DPNS names").rect().left() - LIST_TITLE;
    let keys = words(&kit, "Keys").rect().left() - LIST_TITLE;
    eprintln!("profile {profile}, credits {credits}, names {names}, keys {keys}");
    close(profile, names, "the first column");
    close(credits, keys, "the second column");
}

/// №7: one grid on the pool — «In the pool» and «How it works» are halves, and the actions
/// under them, two in a row at 1024 px, stand in the same two columns.
#[test]
fn pool_cards_stand_on_one_grid() {
    let (_turn, kit) = demo_app("shielded", "en");
    let balance = words(&kit, "In the pool").rect().left() - TITLE_AFTER_ICON;
    let how = words(&kit, "How it works").rect().left() - PLAIN_TITLE;
    let first = button(&kit, "Private address").rect().left();
    let second = button(&kit, "Send privately").rect().left();
    eprintln!("balance {balance}, how {how}, actions {first} and {second}");
    close(balance, first, "the first column");
    close(how, second, "the second column");
}

/// №14: with many identities «Work as» wraps, and the second row of pills begins under the
/// first pill, not under «Work as».
#[test]
fn work_as_wraps_under_the_first_pill() {
    let (_turn, mut kit) = demo_app("identity", "en");
    with_wallet(&mut kit, |w| {
        w.identities.truncate(1);
        for slot in 1..8 {
            w.identities.push(IdentityChoice {
                id: format!("{slot}Kx2LabQ7pWmR9sTzV4nB8cYd1Je3HuF6gNqA2oX7wE"),
                name: Some(format!("evolite-probe-11556721{slot}")),
                slot,
            });
        }
    });
    let label = words(&kit, "Work as").rect();
    let pills: Vec<egui::Rect> = (1..8)
        .map(|slot| button(&kit, &format!("@evolite-probe-11556721{slot}")).rect())
        .collect();
    let first = button(&kit, &shown_name(&kit)).rect();
    let wrapped: Vec<_> = pills.iter().filter(|r| r.top() > first.bottom()).collect();
    assert!(!wrapped.is_empty(), "eight pills wrap at 1024 px");
    let row_start = wrapped.iter().map(|r| r.left()).fold(f32::MAX, f32::min);
    eprintln!("«Work as» {label:?}, the first pill {first:?}, the second row from {row_start}");
    close(row_start, first.left(), "the second row's start");
    assert!(row_start > label.right(), "not under «Work as»");
}

/// №15: «Edit profile» is a text action in the card's top right corner: its words end at the
/// card's right margin, above the name.
#[test]
fn edit_profile_stands_in_the_cards_corner() {
    let (_turn, kit) = demo_app("identity", "en");
    let name = profile_name(&kit);
    let credits = words(&kit, "Platform credits").rect().left() - TITLE_AFTER_ICON;
    let action = kit
        .query_by_label("Edit profile")
        .or_else(|| kit.query_by_label("Create a profile"))
        .expect("the profile's action");
    let words = action
        .children_recursive()
        .find(|n| n.accesskit_node().role() == Role::Label)
        .expect("the action's words")
        .rect();
    // The profile card ends 20 before the credits card; its words end 24 + 1 inside it.
    eprintln!("words {words:?}, name {name:?}, credits card from {credits}");
    close(words.right(), credits - 20.0 - 25.0, "the right margin");
    assert!(words.bottom() <= name.top(), "above the name");
}

/// №16: «Save» in «DashPay profile» is off until something is typed or changed — a write costs
/// credits — and the reason stands beside it.
#[test]
fn an_unchanged_profile_is_not_saved() {
    let (_turn, mut kit) = demo_app("identity", "en");
    let action = if kit.query_by_label("Edit profile").is_some() {
        "Edit profile"
    } else {
        "Create a profile"
    };
    kit.get_by_label(action).click();
    kit.run_steps(3);
    let save = |kit: &Kit| !button(kit, "Save").accesskit_node().is_disabled();
    assert!(!save(&kit), "«Save» with nothing changed");
    assert!(has_text(&kit, "Nothing has changed yet"), "the reason");
    let display = kit
        .state()
        .identity
        .editing
        .as_ref()
        .expect("23 open")
        .display
        .clone();
    let field = kit
        .query_all_by_role(Role::TextInput)
        .find(|n| n.accesskit_node().value().as_deref() == Some(display.as_str()))
        .expect("the «Name» field");
    field.click();
    kit.run_steps(2);
    kit.query_all_by_role(Role::TextInput)
        .find(|n| n.accesskit_node().is_focused())
        .expect("the field takes the typing")
        .type_text("x");
    kit.run_steps(2);
    assert!(save(&kit), "«Save» once the name is changed");
    assert!(!has_text(&kit, "Nothing has changed yet"));
}

/// №27: «Shield funds» opens with an empty amount and no error: nothing is put in for the
/// person, «Max» is beside the field.
#[test]
fn shielding_opens_empty_and_without_an_error() {
    let (_turn, mut kit) = demo_app("shielded", "en");
    wait_for(&mut kit, "«Shield funds» on", |kit| {
        !button(kit, "Shield funds").accesskit_node().is_disabled()
    });
    button(&kit, "Shield funds").click();
    kit.run_steps(3);
    assert!(kit.state().pool.shielding, "the dialog is open");
    let amount = kit.state().pool.amount.clone();
    assert!(amount.is_empty(), "the amount is {amount:?}");
    assert!(!has_text(&kit, "More than there is in this balance"));
    assert!(
        kit.query_by_label("Max").is_some(),
        "«Max» beside the field"
    );
}
