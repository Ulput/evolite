//! FEAT-0008: amounts in the currency of «Fiat rate» on Send, in History and its card — on the
//! demo core, whose rate is 47.9615 USD for a DASH, fetched «now».

use super::*;
use evolite_api::{FiatRate, Snapshot};

/// A label or value with `text` in it.
fn shows(kit: &Kit, text: &str) -> bool {
    has_words(kit, text)
}

/// Frames until `seen`, the snapshot changed by `change` before each.
fn shown_with(
    kit: &mut Kit,
    change: impl Fn(&mut Snapshot),
    what: &str,
    seen: impl Fn(&Kit) -> bool,
) {
    for _ in 0..60 {
        if let Some(s) = kit.state_mut().snapshot.as_mut() {
            change(s);
        }
        kit.run_steps(1);
        if seen(kit) {
            return;
        }
    }
    panic!("not seen: {what}");
}

fn usd_at(price: i64, at: i64) -> impl Fn(&mut Snapshot) {
    move |s: &mut Snapshot| {
        s.fiat = Some(FiatRate {
            currency: "USD".into(),
            price,
            at,
        })
    }
}

fn now() -> i64 {
    chrono::Local::now().timestamp()
}

/// AC-T1: «Fiat rate» off — no unit switch, no «≈ … by the rate», no currency in History.
#[test]
fn with_the_rate_off_send_shows_nothing_new() {
    let (_turn, mut kit) = demo_app("send", "en");
    kit.run_steps(3);
    assert!(kit.query_by_label("USD").is_none(), "no switch");
    assert!(!shows(&kit, "by the rate"));
}

#[test]
fn with_the_rate_off_history_shows_no_currency() {
    let (_turn, mut kit) = demo_app("activity", "en");
    kit.run_steps(3);
    assert!(!shows(&kit, "USD"));
}

/// UC-1, AC-T2: 25 USD typed make the amount in DASH, rounded down, and «Summary» says
/// «≈ 25.00 USD»; the line under the field says the amount is sent in DASH.
#[test]
fn twenty_five_dollars_are_sent_in_dash() {
    let (_turn, mut kit) = demo_app("send-fiat", "en");
    wait_for(&mut kit, "the amount from the currency", |k| {
        !k.state().send.amount.is_empty()
    });
    // 25 USD at 47.9615 = 0.52125142… DASH, down to the duff.
    assert_eq!(kit.state().send.amount, "0.52125142");
    assert!(shows(&kit, "The amount is sent in DASH"));
    assert!(shows(&kit, "≈ 25.00 USD"), "Summary");
    let usd = kit.get_by_label("USD");
    assert_eq!(
        usd.accesskit_node().toggled(),
        Some(egui::accesskit::Toggled::True)
    );
}

/// AC-T3: in the last look a new rate changes neither the amount nor what is signed.
#[test]
fn a_new_rate_in_the_last_look_changes_nothing() {
    let (_turn, mut kit) = demo_app("send-fiat-confirm", "en");
    wait_for(&mut kit, "the amount", |k| {
        !k.state().send.amount.is_empty()
    });
    let before = kit.state().send.amount.clone();
    shown_with(
        &mut kit,
        usd_at(9_000_000_000, now()),
        "the new rate",
        |k| shows(k, "90.00 USD"),
    );
    assert_eq!(kit.state().send.amount, before);
}

/// AC-T4, UC-6: a rate older than 5 minutes takes the currency away and says since when;
/// «Summary» says the rate's time.
#[test]
fn an_old_rate_takes_the_currency_away() {
    let (_turn, mut kit) = demo_app("send-fiat-stale", "en");
    wait_for(&mut kit, "the line", |k| {
        shows(k, "The rate has not been updated since")
    });
    assert!(shows(&kit, "rate at"));
    let usd = kit.get_by_label("USD");
    assert!(usd.accesskit_node().is_disabled());
}

/// AC-T11: the switch is two radio buttons by name; the arrows switch it.
#[test]
fn the_unit_switch_takes_the_arrows() {
    let (_turn, mut kit) = demo_app("send-fiat", "en");
    kit.run_steps(3);
    kit.get_by_label("USD").focus();
    kit.run_steps(1);
    kit.key_press(egui::Key::ArrowLeft);
    kit.run_steps(2);
    assert!(!kit.state().send.in_currency);
    // The amount kept its meaning: the DASH it made is in the field now.
    assert_eq!(kit.state().send.amount, "0.52125142");
}

/// UC-3, UC-4: History says each amount in the currency — at the rate kept, or «now».
#[test]
fn history_says_the_currency_under_each_amount() {
    let (_turn, mut kit) = demo_app("activity-fiat", "en");
    wait_for(&mut kit, "a line in USD", |k| shows(k, "USD · now"));
    // The demo's line 2 kept its rate: 0.25 DASH at 30.78.
    assert!(shows(&kit, "≈ 7.70 USD"));
}

/// Frames 7–8: the card's «In currency» — kept and now apart, or today's said so.
#[test]
fn the_card_says_the_rate_kept_and_now() {
    let (_turn, mut kit) = demo_app("activity-card-fiat", "en");
    wait_for(&mut kit, "In currency", |k| shows(k, "In currency"));
    assert!(shows(&kit, "30.78 USD for a DASH"));
    assert!(shows(&kit, "Now ≈"));
    drop(kit);
    drop(_turn);
    let (_turn, mut kit) = demo_app("activity-card-fiat-old", "en");
    wait_for(&mut kit, "In currency", |k| shows(k, "In currency"));
    assert!(shows(
        &kit,
        "the rate at the time of the operation was not saved"
    ));
}

/// AC-T9: «Hide amounts» hides the currency in the History row and the card too.
#[test]
fn hidden_amounts_hide_the_currency() {
    let (_turn, mut kit) = demo_app("activity-card-fiat", "en");
    kit.state_mut().settings.turn_on("hide-amounts");
    wait_for(&mut kit, "hidden", |k| shows(k, "≈ •••• USD"));
    assert!(!shows(&kit, "≈ 7.70 USD"));
}
