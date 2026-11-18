//! BUG-231 item 3: a contact whose request points to a disabled key (platform-wallet's
//! `payment_channel_broken`) is said in words — on its row, in its card and when it is paid —
//! instead of «try again in a minute».

use super::*;
use crate::screens::Screen;

const TAG: &str = "Needs a new request";
const BY_NAME: &str =
    "You cannot pay @alice by DashPay yet: their contact request points to a disabled key.";
const NAMELESS: &str =
    "You cannot pay this contact by DashPay yet: their contact request points to a disabled key.";
const MINUTE: &str = "try again in a minute";

/// The Contacts list of `preset`, its established contacts drawn.
fn contacts(preset: &str) -> (MutexGuard<'static, ()>, Kit) {
    let (turn, mut kit) = demo_app(preset, "en");
    kit.set_size(egui::vec2(1280.0, 1000.0));
    wait_for(&mut kit, "@alice's row", |kit| {
        kit.query_all_by(|n| n.label().is_some_and(|l| l.starts_with("@alice · ")))
            .next()
            .is_some()
    });
    (turn, kit)
}

/// A row's name: handle, shown name and, for a broken channel, the tag.
fn row_named(kit: &Kit, handle: &str) -> String {
    kit.query_all_by(|n| {
        n.label()
            .is_some_and(|l| l.starts_with(&format!("{handle} · ")))
    })
    .next()
    .and_then(|n| n.accesskit_node().label())
    .unwrap_or_else(|| panic!("{handle}'s row"))
}

/// Send from Core to @alice with an amount: what the form says under the recipient.
fn pay_alice(kit: &mut Kit) {
    kit.state_mut().screen = Screen::Send;
    kit.run_steps(3);
    kit.state_mut().send.recipient = "@alice".into();
    kit.state_mut().send.amount = "0.1".into();
    kit.run_steps(4);
}

fn check_and_send_enabled(kit: &Kit) -> bool {
    kit.query_all_by_role_and_label(Role::Button, "Check and send")
        .next()
        .is_some_and(|n| !n.accesskit_node().is_disabled())
}

/// AC-A2: «Needs a new request» on @alice's row only.
#[test]
fn the_broken_contacts_row_says_it_needs_a_new_request() {
    let (_turn, kit) = contacts("contacts-channel-broken");
    assert!(row_named(&kit, "@alice").ends_with(&format!(" · {TAG}")));
    assert!(has_words(&kit, TAG));
    assert!(!row_named(&kit, "@dash-core-dev").contains(TAG));
}

/// AC-A1: paying @alice is refused by name before the password, not «in a minute».
#[test]
fn paying_the_broken_contact_is_refused_in_words() {
    let (_turn, mut kit) = contacts("contacts-channel-broken");
    pay_alice(&mut kit);
    wait_for(&mut kit, "the refusal", |kit| has_words(kit, BY_NAME));
    assert!(!has_words(&kit, MINUTE));
    assert!(!check_and_send_enabled(&kit), "«Check and send» is enabled");
    assert!(kit.query_all_by_role(Role::PasswordInput).next().is_none());
}

/// R-3: the contact's card has the notice without the name; «Send» stays.
#[test]
fn the_broken_contacts_card_has_the_notice() {
    let (_turn, mut kit) = contacts("contacts-channel-broken");
    let alice = kit
        .state()
        .snapshot
        .as_ref()
        .and_then(|s| s.wallet.as_ref())
        .and_then(|w| {
            w.contacts
                .iter()
                .find(|c| c.name.as_deref() == Some("alice"))
        })
        .map(|c| c.identity_id.clone())
        .expect("@alice");
    kit.state_mut().contacts.profile = Some(alice);
    wait_for(&mut kit, "the notice", |kit| has_words(kit, NAMELESS));
    assert!(
        kit.query_all_by_role_and_label(Role::Button, "Send")
            .any(|n| !n.accesskit_node().is_disabled())
    );
}

/// AC-A3: without the mark there is no tag and @alice can be paid.
#[test]
fn without_the_mark_there_is_no_tag_and_no_refusal() {
    let (_turn, mut kit) = contacts("contacts");
    assert!(!row_named(&kit, "@alice").contains(TAG));
    assert!(!has_words(&kit, TAG));
    pay_alice(&mut kit);
    wait_for(&mut kit, "«Check and send»", check_and_send_enabled);
    assert!(!has_words(&kit, BY_NAME));
}
