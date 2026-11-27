//! FEAT-0038 W10 (UC-13) on the demo core: the Collection's card of one copy — «Orange pin»,
//! bought, no. 1 for sale at 0.5 DASH — offers the Workshop's series windows on that copy:
//! «Give…», «Change the price…», «Take off sale…», or «Sell…» once it is not for sale (AC-A11).

use super::*;

const PIN: &str = "demo-pin-1";

fn pin(kit: &Kit) -> Option<evolite_api::Nft> {
    let snap = kit.state().snapshot.as_ref()?;
    snap.wallet
        .as_ref()?
        .nfts
        .iter()
        .find(|n| n.id == PIN)
        .cloned()
}

/// The Collection with the pin's card open (a window at the kit's size, as the frame W10).
fn open_pin(kit: &mut Kit) {
    wait_for(kit, "the Collection's NFTs", |kit| pin(kit).is_some());
    let at = {
        let snap = kit.state().snapshot.as_ref().unwrap();
        let nfts = &snap.wallet.as_ref().unwrap().nfts;
        nfts.iter().position(|n| n.id == PIN).unwrap()
    };
    let state = &mut kit.state_mut().collection;
    state.selected = at;
    state.open = true;
    // Active once the card has settled (a window takes no clicks while it comes up).
    wait_for(kit, "the pin's card", |kit| {
        kit.query_all_by_label("Give…").next().is_some() && active(kit, "Give…")
    });
}

fn retype(kit: &mut Kit, label: &str, text: &str) {
    kit.get_by_role_and_label(Role::TextInput, label).click();
    kit.run_steps(2);
    for _ in 0..12 {
        kit.key_press(egui::Key::Backspace);
    }
    kit.get_by_role_and_label(Role::TextInput, label)
        .type_text(text);
    kit.run_steps(2);
}

/// The password typed and the window's main button pressed.
fn sign(kit: &mut Kit, label: &str) {
    kit.get_by_role(Role::PasswordInput).click();
    kit.run_steps(2);
    kit.get_by_role(Role::PasswordInput).type_text("demo");
    kit.run_steps(2);
    wait_for(kit, "the button", |kit| {
        !button(kit, label).accesskit_node().is_disabled()
    });
    button(kit, label).click();
}

/// The card's button brought into view (the window scrolls at the kit's size), then clicked.
fn press(kit: &mut Kit, label: &str) {
    button(kit, label).scroll_to_me();
    kit.run_steps(3);
    button(kit, label).click();
}

fn active(kit: &Kit, label: &str) -> bool {
    !button(kit, label).accesskit_node().is_disabled()
}

/// The frame W10: «Price · for sale · 0.5000 DASH», «Give…», «Change the price…», «Take off
/// sale…» active, no «Sell…», and the rules of a direct purchase.
#[test]
fn the_card_of_one_copy_offers_the_series_windows() {
    let (_turn, mut kit) = demo_app("collection", "en");
    open_pin(&mut kit);
    let said: Vec<String> = kit
        .query_all_by(|n| n.label().is_some_and(|l| l.contains("0.5000")))
        .filter_map(|n| n.accesskit_node().label())
        .collect();
    assert!(has_words(&kit, "for sale · 0.5000 DASH"), "{said:?}");
    for label in ["Give…", "Change the price…", "Take off sale…"] {
        assert!(active(&kit, label), "{label}");
    }
    assert!(kit.query_all_by_label("Sell…").next().is_none());
    assert!(has_words(
        &kit,
        "Anyone can buy it — at once, without your confirmation"
    ));
    // The designer's check: the price is the table's first row — above «Owner» — since the
    // buttons are about it and at 800 × 440 the rows below scroll.
    let top = |a: &str, b: &str| {
        // A row says «label: value» — a label's words are its node's value.
        let both = |t: String| says(&t, a) && says(&t, b);
        kit.query_all_by(|n| n.label().is_some_and(both) || n.value().is_some_and(both))
            .map(|n| n.rect().top())
            .fold(f32::INFINITY, f32::min)
    };
    let (price, owner) = (top("Price", "for sale"), top("Owner", "@satoshi"));
    assert!(
        price.is_finite() && price < owner,
        "the price first: {price} vs {owner}"
    );
}

/// UC-13, S4 on one copy: the new price, one record, one password; the card says it after.
#[test]
fn the_one_copys_price_is_changed() {
    let (_turn, mut kit) = demo_app("collection", "en");
    open_pin(&mut kit);
    press(&mut kit, "Change the price…");
    // The designer's check: the window names the copy, not a series.
    wait_for(&mut kit, "S4", |kit| {
        has_words(kit, "“Orange pin” · No. 1 · 0.5000 DASH")
    });
    retype(&mut kit, "New price per copy", "0.75");
    wait_for(&mut kit, "the plan", |kit| has_words(kit, "1 record"));
    assert!(has_words(&kit, "The price changes only on No. 1."));
    assert!(!has_text(&kit, "If all are bought"), "it repeats «Will be»");
    sign(&mut kit, "Sign and change the price");
    wait_for(&mut kit, "the new price", |kit| {
        pin(kit).and_then(|n| n.price) == Some(75_000_000)
    });
    open_pin(&mut kit);
    assert!(has_words(&kit, "for sale · 0.7500 DASH"));
}

/// UC-13, S5 on one copy: no «Which», no «How many»; taken off, the card offers «Sell…».
#[test]
fn the_one_copy_taken_off_sale_can_be_sold_again() {
    let (_turn, mut kit) = demo_app("collection", "en");
    open_pin(&mut kit);
    press(&mut kit, "Take off sale…");
    wait_for(&mut kit, "S5", |kit| {
        has_text(kit, "The copy stays yours; only its price is taken off.")
    });
    assert!(has_words(&kit, "“Orange pin” · No. 1 · 0.5000 DASH"));
    assert!(!has_text(&kit, "Which"));
    assert!(
        kit.query_all_by(
            |n| n.role() == Role::TextInput && n.label().as_deref() == Some("How many")
        )
        .next()
        .is_none()
    );
    wait_for(&mut kit, "the plan", |kit| has_words(kit, "1 record"));
    sign(&mut kit, "Sign and take off sale");
    wait_for(&mut kit, "no price", |kit| {
        pin(kit).is_some_and(|n| n.price.is_none())
    });
    open_pin(&mut kit);
    assert!(has_words(&kit, "not for sale"));
    assert!(active(&kit, "Sell…"));
    assert!(kit.query_all_by_label("Take off sale…").next().is_none());
}

/// AC-A11, R-11: a gift of the copy for sale says it loses its price, has no «How many», and
/// gives that copy — the pin leaves the Collection. Mutant: the gift not of that copy (the free
/// ones only) → nothing to give, red.
#[test]
fn a_gift_of_the_copy_for_sale_warns_and_gives_it() {
    let (_turn, mut kit) = demo_app("collection", "en");
    open_pin(&mut kit);
    press(&mut kit, "Give…");
    wait_for(&mut kit, "W8 on one copy", |kit| {
        has_text(
            kit,
            "No. 1 is for sale — after the gift it will have no price",
        )
    });
    // The designer's check: which copy leaves.
    assert!(has_words(&kit, "No. 1 · for sale at 0.5000 DASH"));
    assert!(
        kit.query_all_by(
            |n| n.role() == Role::TextInput && n.label().as_deref() == Some("How many")
        )
        .next()
        .is_none()
    );
    kit.get_by_role_and_label(Role::TextInput, "Recipient")
        .click();
    kit.run_steps(2);
    kit.get_by_role_and_label(Role::TextInput, "Recipient")
        .type_text("@alice");
    wait_for(&mut kit, "the plan", |kit| has_words(kit, "1 record"));
    sign(&mut kit, "Sign and give");
    wait_for(&mut kit, "the pin given away", |kit| pin(kit).is_none());
}
