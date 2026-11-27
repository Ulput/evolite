//! FEAT-0054: Settings › Advanced › «Add a collection by id», the fallback for a collection the
//! search did not find. A contract that is not of the evolite standard is refused under the
//! field; an id kept is listed with «Remove», and «Remove» takes it out.

use super::*;

const ID: &str = "8rd3Ux7pQfAbCdEfGhJkLmNoPqRsTuVwXyZkV2mJ38R";

fn added(kit: &Kit) -> Vec<String> {
    kit.state()
        .snapshot
        .as_ref()
        .and_then(|s| s.wallet.as_ref())
        .map(|w| w.added_collections.clone())
        .unwrap_or_default()
}

/// The «Open» of the row «Add a collection by id» (the Tools group has two «Open»).
fn open_row(kit: &mut Kit) {
    let row = words(kit, "Add a collection by id").rect();
    let open = kit
        .query_all_by_label("Open")
        .filter(|n| n.accesskit_node().role() == Role::Button)
        .min_by(|a, b| {
            let d = |n: &egui_kittest::Node<'_>| (n.rect().center().y - row.center().y).abs();
            d(a).total_cmp(&d(b))
        })
        .expect("an «Open»");
    open.click();
    kit.run_steps(2);
}

fn type_id(kit: &mut Kit, text: &str) {
    let field = kit.get_by_role(Role::TextInput);
    field.click();
    kit.run_steps(2);
    kit.get_by_role(Role::TextInput).type_text(text);
    kit.run_steps(2);
}

#[test]
fn a_collection_is_added_by_id_refused_when_not_of_the_standard_and_removed() {
    let (_turn, mut kit) = demo_app("advanced", "en");
    kit.set_size(egui::vec2(1024.0, 1100.0));
    kit.state_mut().settings.set("advanced", true);
    wait_for(&mut kit, "the Tools row", |kit| {
        has_text(kit, "Add a collection by id")
    });
    open_row(&mut kit);
    wait_for(&mut kit, "the dialog", |kit| has_text(kit, "Contract id"));

    // Not of the standard: said under the field, nothing kept.
    type_id(&mut kit, evolite_core::DEMO_NOT_STANDARD);
    kit.get_by_role_and_label(Role::Button, "Add").click();
    wait_for(&mut kit, "the refusal", |kit| {
        has_words(kit, "not an NFT collection of the evolite standard")
    });
    assert!(added(&kit).is_empty());

    // A contract id of the standard: listed, the field cleared.
    kit.state_mut().settings.collections.id.clear();
    type_id(&mut kit, ID);
    kit.get_by_role_and_label(Role::Button, "Add").click();
    wait_for(&mut kit, "the id kept", |kit| {
        added(kit) == vec![ID.to_string()]
    });
    wait_for(&mut kit, "its row", |kit| has_text(kit, "Remove"));
    assert!(
        !has_words(&kit, "not an NFT collection"),
        "the refusal is gone"
    );

    kit.get_by_role_and_label(Role::Button, "Remove").click();
    wait_for(&mut kit, "removed", |kit| added(kit).is_empty());
}

/// Only in Advanced mode (the Advanced fallback, not in «Simple», FEAT-0050).
#[test]
fn add_a_collection_by_id_is_only_for_advanced_users() {
    let (_turn, mut kit) = demo_app("advanced", "en");
    wait_for(&mut kit, "Advanced", |kit| has_text(kit, "Advanced mode"));
    assert!(!kit.state().settings.is_on("advanced"));
    assert!(!has_text(&kit, "Add a collection by id"));
}
