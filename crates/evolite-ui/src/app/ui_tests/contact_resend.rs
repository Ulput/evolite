//! FEAT-0020-resend on the screens (the analyst's AC-A1–A5, the architect's AC-T1): replacing
//! the DashPay pair says before the password what the requests sent again will cost; the demo
//! core sends them and writes one line; what is still owed shows on K1 and in Contacts, with
//! «Send requests again»; a contact no request can go to is said so, with no button.
//!
//! Every edit of the snapshot goes through `show_wallet_edited` / `draw_edited`
//! (TECH-kittest-snapshot-guard): the demo core's own snapshot would undo it otherwise.

use super::*;
use crate::screens::Screen;
use evolite_api::{Contact, ContactState, Duffs, Profile, Resend, Wallet};

fn contact(n: u8) -> Contact {
    Contact {
        identity_id: format!("{n}{}", "C".repeat(43)),
        name: Some(format!("friend{n}")),
        profile: Profile::default(),
        state: ContactState::Established,
        since: 1_790_000_000,
        payments: 0,
        last_payment: None,
        channel_broken: false,
    }
}

/// The shown identity with `n` established contacts, `credits` of credits, and `resend`.
fn setup(n: u8, credits: Duffs, resend: Option<Resend>) -> impl Fn(&mut Wallet) + Clone {
    move |w: &mut Wallet| {
        w.contacts = (1..=n).map(contact).collect();
        w.balances.credits = credits;
        w.balances.credits_checked = true;
        w.resend = resend.clone();
    }
}

/// [`setup`] put into the app before each of `frames` frames, while the test acts.
fn keep(kit: &mut Kit, frames: usize, edit: impl Fn(&mut Wallet) + Clone) {
    draw_edited(kit, frames, move |app| {
        if let Some(w) = app.snapshot.as_mut().and_then(|s| s.wallet.as_mut()) {
            edit(w);
        }
    });
}

fn k1(preset: &str) -> (std::sync::MutexGuard<'static, ()>, Kit) {
    let (turn, mut kit) = demo_app(preset, "en");
    kit.set_size(egui::vec2(1280.0, 1100.0));
    wait_for(&mut kit, "K1", |k| k.state().screen == Screen::IdentityKeys);
    kit.run_steps(3);
    (turn, kit)
}

fn open_pair(kit: &mut Kit) {
    kit.get_by_role_and_label(Role::Button, "Key 6 — actions")
        .click();
    kit.run_steps(3);
    button(kit, "Replace the DashPay pair…").click();
    kit.run_steps(3);
}

fn per_request(kit: &Kit) -> Duffs {
    kit.state()
        .snapshot
        .as_ref()
        .and_then(|s| s.wallet.as_ref())
        .map_or(0, |w| w.fees.contact_request)
}

/// AC-T1, AC-A1, AC-A3: the pair replaced with 3 contacts — before the password, how many,
/// the requests' fee and the total with the change; after it, the demo core sends a request to
/// each of its own established contacts and History has one line «N of N», K1 says so.
#[test]
fn replacing_the_pair_resends_to_every_contact() {
    let (_turn, mut kit) = k1("identity-keys");
    let each = per_request(&kit);
    open_pair(&mut kit);
    show_wallet_edited(
        &mut kit,
        2,
        "the requests' block",
        setup(3, 10_000_000, None),
        |k| has_words(k, "Contacts: 3"),
    );
    assert!(has_words(&kit, "evolite will send new contact requests"));
    let fee = crate::format::amount(3 * each);
    assert!(has_words(&kit, &format!("Fee for the requests ≈ {fee}")));
    let total = crate::format::amount(80_000 + 3 * each);
    assert!(has_words(
        &kit,
        &format!("Total with the key change ≈ {total}")
    ));
    kit.get_by_role(Role::PasswordInput).type_text("demo");
    kit.run_steps(3);
    button(&kit, "Replace the pair").click();
    wait_for(&mut kit, "the batch's line", |k| {
        k.state()
            .snapshot
            .as_ref()
            .and_then(|s| s.wallet.as_ref())
            .and_then(|w| w.activity.first())
            .is_some_and(|a| {
                matches!(
                    a.kind,
                    evolite_api::ActivityKind::ContactsResent { sent, of, left: 0 }
                        if sent == of && sent > 0
                )
            })
    });
    kit.run_steps(3);
    assert!(has_words(&kit, "Contact requests sent again: "));
}

/// AC-A1: without established contacts there is nothing to send again — no block.
#[test]
fn no_contacts_no_requests_block() {
    let (_turn, mut kit) = k1("identity-keys");
    open_pair(&mut kit);
    show_wallet_edited(
        &mut kit,
        3,
        "no contacts",
        setup(0, 10_000_000, None),
        |k| has_text(k, "Replace the DashPay pair"),
    );
    assert!(!has_words(&kit, "evolite will send new contact requests"));
    assert!(!has_words(&kit, "Contacts: "));
}

/// AC-A2: credits for the change and 1 of 3 requests — said, and the change still goes.
#[test]
fn credits_for_one_of_three_requests_are_said_and_the_change_goes() {
    let (_turn, mut kit) = k1("identity-keys");
    let each = per_request(&kit);
    let edit = setup(3, 80_000 + each, None);
    open_pair(&mut kit);
    show_wallet_edited(&mut kit, 2, "1 of 3", edit.clone(), |k| {
        has_words(
            k,
            "There are credits for the key change and 1 of 3 requests",
        )
    });
    kit.get_by_role(Role::PasswordInput).type_text("demo");
    keep(&mut kit, 3, edit);
    assert!(
        !button(&kit, "Replace the pair")
            .accesskit_node()
            .is_disabled()
    );
}

/// AC-A4: one contact owed — K1's plate and «Send requests again (1)»; the window asks the
/// password and sends; once the core says so and nothing is owed, the plate and the window go.
#[test]
fn what_is_owed_is_sent_again_from_k1() {
    let (_turn, mut kit) = k1("identity-keys");
    let owed = setup(
        3,
        10_000_000,
        Some(Resend {
            due: vec![contact(2).identity_id],
            ..Resend::default()
        }),
    );
    show_wallet_edited(&mut kit, 2, "the plate", owed.clone(), |k| {
        has_words(k, "Not every contact got a new request — 1 left")
    });
    button(&kit, "Send requests again (1)").click();
    keep(&mut kit, 3, owed.clone());
    assert!(has_words(&kit, "Contacts: 1"));
    kit.get_by_role(Role::PasswordInput).type_text("demo");
    keep(&mut kit, 3, owed);
    button(&kit, "Sign and send").click();
    // Sent: the core answers. The demo core keeps its own wallet, with nothing owed, so the
    // answer and the next snapshot of a core that had the debt are given here.
    wait_for(&mut kit, "the core's answer", |k| {
        k.state()
            .identity_keys
            .resend
            .as_ref()
            .is_some_and(|w| w.flow.result().is_some())
    });
    if let Some(window) = &mut kit.state_mut().identity_keys.resend {
        window
            .flow
            .set_stage(crate::components::spend_flow::Stage::Answered {
                signed: (),
                result: Ok(()),
            });
    }
    show_wallet_edited(
        &mut kit,
        2,
        "nothing owed",
        setup(3, 10_000_000, None),
        |k| !has_words(k, "Not every contact got a new request"),
    );
    assert!(
        kit.state().identity_keys.resend.is_none(),
        "the window closed"
    );
}

/// AC-A4 in Contacts: the owed contact's row says «Waiting for your request», another's does
/// not; its card has the plate and «Send the request again».
#[test]
fn an_owed_contact_is_marked_in_contacts() {
    let (_turn, mut kit) = demo_app("contacts", "en");
    kit.set_size(egui::vec2(1280.0, 1100.0));
    wait_for(&mut kit, "Contacts", |k| {
        k.state().screen == Screen::Contacts
    });
    let owed = setup(
        2,
        10_000_000,
        Some(Resend {
            due: vec![contact(2).identity_id],
            ..Resend::default()
        }),
    );
    show_wallet_edited(&mut kit, 2, "the mark", owed.clone(), |k| {
        has_words(k, "friend2 · Waiting for your request")
    });
    assert!(!has_words(&kit, "friend1 · Waiting for your request"));
    kit.state_mut().contacts.profile = Some(contact(2).identity_id);
    keep(&mut kit, 3, owed.clone());
    assert!(has_words(
        &kit,
        "This contact cannot pay you until you send them a new request."
    ));
    button(&kit, "Send the request again").click();
    keep(&mut kit, 3, owed);
    assert!(kit.state().contacts.resend.is_some(), "the window for one");
}

/// AC-A5: a contact no request can go to (R-11) — said on K1, no «Send requests again».
#[test]
fn a_blocked_contact_has_words_and_no_button() {
    let (_turn, mut kit) = k1("identity-keys");
    let blocked = setup(
        2,
        10_000_000,
        Some(Resend {
            blocked: vec![contact(1).identity_id],
            ..Resend::default()
        }),
    );
    show_wallet_edited(&mut kit, 2, "R-11", blocked, |k| {
        has_words(
            k,
            "A new request cannot be sent to this contact: the limit of repeated requests is used up",
        )
    });
    assert!(kit.query_by_label("Send requests again (1)").is_none());
    assert!(kit.query_by_label("Send requests again (0)").is_none());
}
