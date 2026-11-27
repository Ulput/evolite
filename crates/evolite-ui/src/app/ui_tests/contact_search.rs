//! FEAT-0057 П4: the search in Contacts — only from 9 requests and contacts (З-2); by @name, a
//! word of the display name or the id, with «N of M» in each section's heading (UC-4.2); nothing
//! found, said, with «Find @… in DPNS» (UC-4.3); «×» clears it (UC-4.4); another identity
//! clears it (A4.4).

use super::*;
use crate::screens::contacts::SEARCH_FROM;

const SEARCH: &str = "Name, @name or id";

/// The demo's contacts and requests, with as many more contacts as make `total`: copies of an
/// established one, each its own name, display name and id.
fn grown(total: usize) -> impl Fn(&mut evolite_api::Wallet) {
    move |w| {
        let Some(model) = w
            .contacts
            .iter()
            .find(|c| c.state != evolite_api::ContactState::Incoming)
            .cloned()
        else {
            return;
        };
        let mut n = 0;
        while w.contacts.len() < total {
            let mut c = model.clone();
            c.identity_id = format!("Zq{n:02}xExtraContactIdentity");
            c.name = Some(format!("extra{n:02}"));
            c.profile.display = Some(format!("Extra Person {n:02}"));
            w.contacts.push(c);
            n += 1;
        }
    }
}

fn contacts(total: usize) -> (std::sync::MutexGuard<'static, ()>, Kit) {
    let (turn, mut kit) = demo_app("contacts", "en");
    settle(&mut kit);
    show_wallet_edited(&mut kit, 2, "the contacts", grown(total), |_| true);
    (turn, kit)
}

fn counts(kit: &Kit) -> (usize, usize) {
    let w = kit
        .state()
        .snapshot
        .as_ref()
        .and_then(|s| s.wallet.as_ref())
        .unwrap();
    let requests = w
        .contacts
        .iter()
        .filter(|c| c.state == evolite_api::ContactState::Incoming)
        .count();
    (requests, w.contacts.len() - requests)
}

/// Draws `query` as typed, over the grown list.
fn search(kit: &mut Kit, total: usize, query: &str) {
    kit.state_mut().contacts.search = query.into();
    show_wallet_edited(kit, 2, "the search", grown(total), |_| true);
}

/// З-2: below 9 requests and contacts no field; from 9, there it is.
#[test]
fn the_search_is_there_only_from_nine() {
    let (_turn, kit) = contacts(SEARCH_FROM - 1);
    assert!(!has_text(&kit, SEARCH), "8: no search");
    drop(kit);
    drop(_turn);
    let (_turn, kit) = contacts(SEARCH_FROM);
    assert!(has_text(&kit, SEARCH), "9: the search");
}

/// UC-4.2: by @name from its start, by a word of the display name, by the id from its start;
/// the headings say «N of M».
#[test]
fn the_search_finds_by_name_display_name_and_id() {
    let (_turn, mut kit) = contacts(14);
    let (requests, known) = counts(&kit);
    search(&mut kit, 14, "@extra0");
    assert!(
        has_words(&kit, &format!("Contacts · 10 of {known}")),
        "@name: 10 of the extras"
    );
    assert!(has_words(&kit, "@extra03"));
    assert!(!has_words(&kit, "Alice Chen"), "alice is not shown");
    assert!(!has_words(&kit, "Requests ·"), "no request matches");

    search(&mut kit, 14, "chen");
    assert!(
        has_words(&kit, &format!("Contacts · 1 of {known}")),
        "a word of the display name"
    );
    assert!(has_words(&kit, "Alice Chen"));

    search(&mut kit, 14, "3Ah7");
    assert!(
        has_words(&kit, &format!("Contacts · 1 of {known}")),
        "the id's start"
    );
    search(&mut kit, 14, "h7Kp");
    assert!(!has_words(&kit, "Alice Chen"), "not the id's middle");

    search(&mut kit, 14, "");
    assert!(
        has_words(&kit, &format!("Contacts · {known}")),
        "all, no «of»"
    );
    if requests > 0 {
        assert!(has_words(&kit, &format!("Requests · {requests}")));
    }
}

/// UC-4.3: nothing found — said, and «Find @… in DPNS» opens «Add a contact» with that name.
#[test]
fn nothing_found_offers_the_name_in_dpns() {
    let (_turn, mut kit) = contacts(12);
    search(&mut kit, 12, "@carol");
    assert!(has_words(&kit, "No contacts match \"@carol\""));
    button(&kit, "Find @carol in DPNS").click();
    show_wallet_edited(&mut kit, 2, "the click", grown(12), |_| true);
    let adding = kit.state().contacts.adding.as_ref().map(|a| a.name.clone());
    assert_eq!(adding.as_deref(), Some("@carol"));

    kit.state_mut().contacts.adding = None;
    search(&mut kit, 12, "no such name!");
    assert!(has_words(&kit, "No contacts match"));
    assert!(
        kit.query_all_by_label("Find @no such name! in DPNS")
            .next()
            .is_none(),
        "not a name: no DPNS button"
    );
}

/// UC-4.4: «×» clears the search.
#[test]
fn the_cross_clears_the_search() {
    let (_turn, mut kit) = contacts(12);
    search(&mut kit, 12, "chen");
    button(&kit, "Clear the search").click();
    show_wallet_edited(&mut kit, 2, "the click", grown(12), |_| true);
    assert!(kit.state().contacts.search.is_empty());
    assert!(has_words(&kit, "Alice Chen"));
    assert!(
        kit.query_all_by_label("Clear the search").next().is_none(),
        "no «×» when empty"
    );
}

/// The field is typed into as a person does.
#[test]
fn typing_in_the_field_filters() {
    let (_turn, mut kit) = contacts(12);
    kit.get_by_role_and_label(Role::TextInput, SEARCH).click();
    show_wallet_edited(&mut kit, 2, "the focus", grown(12), |_| true);
    kit.get_by_role_and_label(Role::TextInput, SEARCH)
        .type_text("chen");
    show_wallet_edited(&mut kit, 2, "the typing", grown(12), |_| true);
    assert_eq!(kit.state().contacts.search, "chen");
    assert!(has_words(&kit, "Alice Chen"));
    assert!(!has_words(&kit, "@extra01"));
}

/// A4.4: another identity — the search is cleared.
#[test]
fn another_identity_clears_the_search() {
    let (_turn, mut kit) = contacts(12);
    search(&mut kit, 12, "chen");
    show_wallet_edited(
        &mut kit,
        2,
        "another identity",
        |w| {
            grown(12)(w);
            if let Some(i) = w.identity.as_mut() {
                i.id = "AnotherIdentityOfThisWallet".into();
            }
        },
        |_| true,
    );
    assert!(kit.state().contacts.search.is_empty());
}
