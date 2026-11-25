//! FEAT-0057 П5 (BUG-292): a contact's card — the two dates, what was paid each way with how
//! many payments, one date for a request one way, the amounts hidden with «Hide amounts», and
//! «still looking for payments…» while the contacts' accounts are rescanned.

use super::*;

const ALICE: &str = "3Ah7Kp2Qx9Lm4Vt8Rn5Wc1Jd6Yb2Fs7Ge4Hu3Mz9Lp1";
const DASH_CORE_DEV: &str = "6Hk2Ft9wQmRz8LpXcV4nB7sYd3Ke1JaPu5q9W5";
const KATE: &str = "2Lx8Vb5Nq3Rt7Wm1Kc9Hd4Fj6Ys2Pg8Tr4N";

fn card(id: &str) -> (std::sync::MutexGuard<'static, ()>, Kit) {
    let (turn, mut kit) = demo_app("contacts", "en");
    settle(&mut kit);
    kit.state_mut().contacts.profile = Some(id.into());
    settle(&mut kit);
    (turn, kit)
}

/// A5.1, A5.2: both dates, and each way its sum with the count of payments (the demo: 0.35 in 3
/// out, 0.25 in 1 in); no «In contacts since».
#[test]
fn a_contacts_card_says_both_dates_and_each_ways_payments() {
    let (_turn, kit) = card(ALICE);
    for words in [
        "You added",
        "They added",
        "Time of the write in Platform",
        "0.3500 DASH · 3 payments",
        "0.2500 DASH · 1 payment",
    ] {
        assert!(has_words(&kit, words), "«{words}»");
    }
    assert!(!has_words(&kit, "since"), "no third date");
}

/// A5.2: no payment one way — «0.0000 DASH», no count.
#[test]
fn no_payment_one_way_is_a_zero_without_a_count() {
    let (_turn, kit) = card(DASH_CORE_DEV);
    assert!(has_words(&kit, "0.1000 DASH · 1 payment"), "sent");
    assert!(has_text(&kit, "Received: 0.0000 DASH") || has_words(&kit, "0.0000 DASH"));
    assert!(!has_words(&kit, "0.0000 DASH ·"), "no count by a zero");
}

/// A5.3: a request one way — its one date, no sums.
#[test]
fn a_request_one_way_has_one_date_and_no_sums() {
    let (_turn, kit) = card(KATE);
    assert!(has_words(&kit, "Request sent"));
    assert!(!has_words(&kit, "They added"));
    assert!(!has_words(&kit, "Received"), "no sums");
}

/// A5.4, A5.5: «Hide amounts» hides the sums, not the dates; while the contacts' accounts are
/// rescanned, «still looking for payments…».
#[test]
fn hidden_amounts_and_the_rescan_are_said_on_the_card() {
    let (_turn, mut kit) = card(ALICE);
    kit.state_mut().settings.flip_hide_amounts();
    kit.run_steps(3);
    assert!(!has_words(&kit, "0.3500 DASH"), "the sum hidden");
    assert!(has_words(&kit, "You added"), "the dates stay");
    kit.state_mut().settings.flip_hide_amounts();
    if let Some(s) = kit.state_mut().snapshot.as_mut() {
        s.sync.contacts_rescan = true;
    }
    kit.run_steps(1);
    assert!(has_words(&kit, "still looking for payments"));
}

/// BUG-292 in the list (r2, Must 2): «contact with» is the date both had a request, the later
/// one (the demo: four days after ours), not the earlier (the mutant: `since`, the earlier).
#[test]
fn the_list_says_the_later_date_both_had_a_request() {
    let (_turn, mut kit) = demo_app("contacts", "en");
    settle(&mut kit);
    let alice = kit
        .state()
        .snapshot
        .as_ref()
        .and_then(|s| s.wallet.as_ref())
        .and_then(|w| w.contacts.iter().find(|c| c.identity_id == ALICE))
        .cloned()
        .expect("the demo's @alice");
    let now = chrono::Local::now();
    let later = crate::text::date_day(alice.record.they_added.unwrap(), now);
    let earlier = crate::text::date_day(alice.record.you_added.unwrap(), now);
    assert_ne!(later, earlier);
    // The line under the name, as the row paints it (painted, not in the tree).
    let line = crate::screens::contacts::about_line(&alice, now);
    assert!(line.contains(&format!("contact with {later}")), "{line}");
    assert!(!line.contains(&earlier), "not the earlier: {line}");
    assert!(painted_has(&kit, "contact with"), "the row draws it");
    // No date known: no date, not another one in its place.
    let mut unknown = alice;
    unknown.record.you_added = None;
    unknown.record.they_added = None;
    let line = crate::screens::contacts::about_line(&unknown, now);
    assert!(!line.contains("contact with"), "{line}");
}

/// Must 3: a fee not known — «≈» before the sent sum, the count as it is.
#[test]
fn a_sent_sum_with_a_fee_not_known_is_approximate() {
    let (_turn, mut kit) = card(ALICE);
    show_wallet_edited(
        &mut kit,
        2,
        "a fee not known",
        |w| {
            if let Some(c) = w.contacts.iter_mut().find(|c| c.identity_id == ALICE) {
                c.record.sent_exact = false;
            }
        },
        |_| true,
    );
    assert!(has_words(&kit, "≈ 0.3500 DASH · 3 payments"));
    assert!(!has_words(&kit, "≈ 0.2500"), "the received sum is exact");
}
