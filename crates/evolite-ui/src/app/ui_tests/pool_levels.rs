//! FEAT-0049 (the customer's Q15, Q16): one pool screen for both levels; «Unshield» to the
//! wallet's own Core balance for everyone, chosen by default; the Platform address and the
//! notes count only at the advanced level.

use super::*;

/// Q16: the dialog opens on «To your Core balance», with «Arrives on Core in a few minutes»;
/// at the simple level there is no Platform address to unshield to.
#[test]
fn unshield_goes_to_core_by_default() {
    let (_turn, mut kit) = demo_app("unshield", "en");
    wait_for(&mut kit, "the dialog", |kit| {
        has_text(kit, "Unshield from the pool")
    });
    assert!(has_text(&kit, "To your Core balance"));
    assert!(has_text(&kit, "Arrives on Core in a few minutes"));
    assert!(
        kit.query_all_by_label("Platform address").next().is_none(),
        "the simple level has no Platform address"
    );
}

/// Q15: in the «Advanced» mode, the Platform address is a third place to go.
#[test]
fn the_advanced_level_can_unshield_to_the_platform_address() {
    let (_turn, mut kit) = demo_app("unshield", "en");
    kit.state_mut().settings.set("advanced", true);
    wait_for(&mut kit, "the third place", |kit| {
        kit.query_all_by_label("Platform address").next().is_some()
    });
    assert!(has_text(&kit, "To your Core balance"));
}

/// Q15: the pool screen at the simple level says nothing of notes nor of a Platform address;
/// at the advanced level its lines are as before.
#[test]
fn the_pool_screen_by_level() {
    let (_turn, mut kit) = demo_app("shielded", "en");
    wait_for(&mut kit, "the screen", |kit| has_text(kit, "How it works"));
    assert!(painted_has(&kit, "to your Core balance"));
    assert!(painted_has(&kit, "from Core or identity credits"));
    assert!(!has_words(&kit, " notes"), "{:?}", painted_texts(&kit));
    kit.state_mut().settings.set("advanced", true);
    wait_for(&mut kit, "the advanced lines", |kit| {
        painted_has(kit, "to Core, an identity or a Platform address")
    });
}
