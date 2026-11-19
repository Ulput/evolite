//! BUG-179, BUG-180, BUG-181 (QA pass 26): the claim and purchase windows keep what was signed,
//! the credits row does not overlap its label, and «Sign and buy» on an old price buys on one
//! click when the price did not change.

use super::*;

/// The demo's Evo Coffee changed as `edit` says, kept until frames are drawn with it.
fn with_row(kit: &mut Kit, what: &str, edit: impl Fn(&mut evolite_api::tokens::TokenRow)) {
    show_wallet_edited(
        kit,
        2,
        what,
        |w| {
            if let Some(row) = w.tokens.rows.iter_mut().find(|r| r.offer.is_some()) {
                edit(row);
            }
        },
        |_| true,
    );
}

/// The «Credits of …» row: where it is drawn and what it says.
fn credits_row(kit: &Kit) -> (egui::Rect, String) {
    let row = kit
        .query_all_by(|n| {
            n.label()
                .or_else(|| n.value())
                .is_some_and(|w| w.starts_with("Credits of"))
        })
        .next()
        .expect("the credits row");
    let node = row.accesskit_node();
    let words = node.label().or_else(|| node.value()).unwrap_or_default();
    (row.rect(), words.to_string())
}

/// The identity's credits as read (the credits row shows only then).
fn credits_read(kit: &mut Kit) {
    wait_for(kit, "the wallet", |kit| {
        kit.state()
            .snapshot
            .as_ref()
            .is_some_and(|s| s.wallet.is_some())
    });
    show_wallet_edited(
        kit,
        1,
        "the credits read",
        |w| w.balances.credits_read = true,
        |_| true,
    );
}

/// BUG-179: claimed, the next distribution in the snapshot: the window still says what was
/// claimed, not the next one.
#[test]
fn a_claim_done_keeps_what_was_signed() {
    let (_turn, mut kit) = demo_app("tokens-claim", "en");
    wait_for(&mut kit, "K1", |kit| {
        has_words(kit, "Accrued: at least 24.00")
    });
    button(&kit, "Sign and claim").click();
    // One frame: sent; the demo's refusal comes later and nobody waits for it then.
    kit.run_steps(1);
    kit.state_mut().tokens.on_claimed(&Ok(()));
    with_row(&mut kit, "the next distribution", |row| {
        if let Some(p) = row.claims.as_mut().and_then(|c| c.perpetual.as_mut()) {
            p.at_least = Some(21_000);
        }
    });
    kit.run_steps(1);
    assert!(has_words(
        &kit,
        "Claimed: the amount is in the balance and in History."
    ));
    assert!(
        has_words(&kit, "Accrued: at least 24.00"),
        "what was claimed"
    );
    assert!(!has_words(&kit, "at least 210.00"), "not the next one");
}

/// BUG-179: bought, the credits after it in the snapshot: «Credits of …» stays as signed, not
/// reckoned as another purchase.
#[test]
fn a_purchase_done_keeps_its_credits_as_signed() {
    let (_turn, mut kit) = demo_app("tokens-buy", "en");
    credits_read(&mut kit);
    wait_for(&mut kit, "P1", |kit| has_text(kit, "Buy Evo Coffee"));
    kit.run_steps(3);
    let before = |kit: &Kit| credits_row(kit).1;
    let signed = before(&kit);
    button(&kit, "Sign and buy").click();
    kit.run_steps(1);
    kit.state_mut().tokens.on_bought(&Ok(()));
    // As much as the credits were, less the purchase: put in again over a later snapshot,
    // never taken twice.
    let after = kit
        .state()
        .snapshot
        .as_ref()
        .and_then(|s| s.wallet.as_ref())
        .map_or(0, |w| w.balances.credits)
        - 1_000_000;
    show_wallet_edited(
        &mut kit,
        3,
        "the credits after the purchase",
        |w| w.balances.credits = after,
        |_| true,
    );
    assert!(has_words(&kit, "Bought: the tokens are in the balance."));
    assert_eq!(before(&kit), signed, "as signed");
}

/// BUG-180: a long `@name` and both sums: the value goes under the label, the row two lines high.
#[test]
fn the_credits_row_does_not_overlap_its_label() {
    let (_turn, mut kit) = demo_app("tokens-buy", "en");
    wait_for(&mut kit, "P1", |kit| has_text(kit, "Buy Evo Coffee"));
    // BUG-187: the credits read and the long name are put into the snapshot by hand, and a
    // snapshot the demo core sent later took them away under a loaded run. First the core goes
    // quiet; then they are put in, again whenever a snapshot replaced them, until the row says
    // the long name — bounded by `PATIENCE`, not a count of frames.
    settle(&mut kit);
    let long = |w: &mut evolite_api::Wallet| {
        w.balances.credits_read = true;
        if let Some(i) = w.identity.as_mut() {
            i.names = vec![evolite_api::DpnsName {
                label: "qa-kestrel-1108b-a-long-name".into(),
                primary: true,
                state: evolite_api::NameState::Registered,
                contest: None,
            }];
        }
    };
    let shown = |kit: &Kit| {
        kit.query_all_by(|n| {
            n.label()
                .or_else(|| n.value())
                .is_some_and(|w| w.starts_with("Credits of") && w.contains("a-long-name"))
        })
        .next()
        .is_some()
    };
    show_wallet_edited(&mut kit, 1, "«Credits of» with the long name", long, shown);
    let (rect, _) = credits_row(&kit);
    assert!(rect.height() > 30.0, "{rect:?}");
}

/// BUG-181: «Sign and buy» on a price over a minute old reads it again and, the price the same,
/// buys on that one click (the demo then refuses, as it refuses every spend).
#[test]
fn an_old_price_unchanged_buys_on_one_click() {
    let (_turn, mut kit) = demo_app("tokens-buy", "en");
    wait_for(&mut kit, "P1", |kit| has_text(kit, "Buy Evo Coffee"));
    // An old price, not asked again yet.
    with_row(&mut kit, "an old price", |row| {
        if let Some(offer) = row.offer.as_mut() {
            offer.read_ms = 1_000;
        }
    });
    if let Some(buy) = &mut kit.state_mut().tokens.buying {
        buy.asked_price = true;
    }
    kit.run_steps(2);
    button(&kit, "Sign and buy").click();
    wait_for(&mut kit, "the purchase sent", |kit| {
        has_words(kit, "Not bought")
    });
}

/// BUG-181 AC-T1: the price read again decides by the total: the same or less buys, more is
/// said, what cannot be bought is refused.
#[test]
fn a_price_read_again_decides_by_the_total() {
    use crate::screens::tokens::actions::{Reread, after_reread};
    use evolite_api::tokens::Quote;
    let total = |total| {
        Some(Quote::Total {
            total,
            per_unit: 10,
            tier: 1,
        })
    };
    assert_eq!(
        after_reread(100, total(100), false),
        Reread::Buy,
        "the same"
    );
    assert_eq!(
        after_reread(100, total(80), false),
        Reread::Buy,
        "cheaper, a better tier"
    );
    assert_eq!(
        after_reread(100, total(120), false),
        Reread::Dearer { now: 120, was: 100 },
        "dearer, a worse tier"
    );
    assert_eq!(
        after_reread(100, total(100), true),
        Reread::Refused,
        "paused, the limit"
    );
    assert_eq!(
        after_reread(100, Some(Quote::UnderMinimum { minimum: 500 }), false),
        Reread::Refused,
        "under the new minimum"
    );
    assert_eq!(
        after_reread(100, None, false),
        Reread::Refused,
        "not sold any more"
    );
}

/// Now, as the window's clock reads it, in ms.
fn now_ms() -> u64 {
    chrono::Utc::now().timestamp_millis().max(0) as u64
}

/// The count typed in the purchase window, in the token's smallest units.
fn typed_count(kit: &mut Kit) -> u64 {
    let decimals = kit
        .state()
        .snapshot
        .as_ref()
        .and_then(|s| s.wallet.as_ref())
        .and_then(|w| w.tokens.rows.iter().find(|r| r.offer.is_some()))
        .expect("the demo's Evo Coffee")
        .info
        .decimals;
    let typed = kit
        .state()
        .tokens
        .buying
        .as_ref()
        .map(|b| b.count.clone())
        .unwrap_or_default();
    crate::screens::tokens::state::typed(&typed, decimals).expect("a count")
}

/// «Sign and buy» pressed `ago_ms` ago on the old price showing `shown`, for the count typed.
fn press_pending(kit: &mut Kit, shown: u64, ago_ms: u64) {
    let count = typed_count(kit);
    if let Some(buy) = &mut kit.state_mut().tokens.buying {
        buy.pending = Some(crate::screens::tokens::actions::Pending {
            total: shown,
            count,
            at_ms: now_ms().saturating_sub(ago_ms),
        });
    }
}

/// The purchase window of 15 Evo Coffee, its price read again just now, «Sign and buy» pressed
/// on the old one showing `shown` (BUG-181 AC-T2).
fn read_again(shown: u64) -> (MutexGuard<'static, ()>, Kit) {
    let (turn, mut kit) = demo_app("tokens-buy", "en");
    wait_for(&mut kit, "P1", |kit| has_text(kit, "Buy Evo Coffee"));
    kit.run_steps(2);
    press_pending(&mut kit, shown, 0);
    kit.run_steps(2);
    (turn, kit)
}

/// BUG-181 r2, Should 1: no fresh price 30 s after the click (no answer, or the read refused)
/// — «Could not check the price», nothing sent, «Sign and buy» on again.
#[test]
fn a_price_not_read_again_in_time_is_given_up() {
    let (_turn, mut kit) = demo_app("tokens-buy", "en");
    wait_for(&mut kit, "P1", |kit| has_text(kit, "Buy Evo Coffee"));
    kit.run_steps(2);
    // The price still old, asked already: the answer never came.
    with_row(&mut kit, "an old price", |row| {
        if let Some(offer) = row.offer.as_mut() {
            offer.read_ms = 1_000;
        }
    });
    if let Some(buy) = &mut kit.state_mut().tokens.buying {
        buy.asked_price = true;
    }
    press_pending(&mut kit, u64::MAX / 4, 31_000);
    kit.run_steps(3);
    assert!(has_words(&kit, "Could not check the price — try again."));
    let buy = kit.state().tokens.buying.as_ref().expect("the window");
    assert!(buy.pending.is_none(), "given up");
    assert!(!buy.waiting && buy.result.is_none(), "nothing sent");
    assert!(
        !button(&kit, "Sign and buy").accesskit_node().is_disabled(),
        "on again"
    );
}

/// BUG-181 r2, Should 2: pressed on 15, another count typed before the price came back — that
/// count was never shown a total, so nothing is sent.
#[test]
fn another_count_typed_while_checking_sends_nothing() {
    let (_turn, mut kit) = demo_app("tokens-buy", "en");
    wait_for(&mut kit, "P1", |kit| has_text(kit, "Buy Evo Coffee"));
    kit.run_steps(2);
    // Cheaper than anything: with the count kept it would buy at once.
    press_pending(&mut kit, u64::MAX / 4, 0);
    if let Some(buy) = &mut kit.state_mut().tokens.buying {
        buy.count = "5".into();
    }
    kit.run_steps(4);
    let buy = kit.state().tokens.buying.as_ref().expect("the window");
    assert!(buy.pending.is_none(), "dropped");
    assert!(!buy.waiting && buy.result.is_none(), "nothing sent");
    assert!(!has_words(&kit, "Not bought"));
}

/// BUG-181 AC-T2: cheaper than shown — bought on that click (the demo refuses, as every spend).
#[test]
fn an_old_price_now_cheaper_buys_on_that_click() {
    let (_turn, mut kit) = read_again(u64::MAX / 4);
    wait_for(&mut kit, "the purchase sent", |kit| {
        has_words(kit, "Not bought")
    });
}

/// BUG-181 AC-T2: dearer than shown — nothing sent, «now … (was …)», the password kept.
#[test]
fn an_old_price_now_dearer_stops_and_says_so() {
    let (_turn, kit) = read_again(1);
    assert!(has_words(&kit, "The price grew: now"), "said");
    let buy = kit.state().tokens.buying.as_ref().expect("the window");
    assert!(!buy.waiting && buy.result.is_none(), "nothing sent");
    assert!(buy.dearer.is_some());
    assert!(
        !button(&kit, "Sign and buy").accesskit_node().is_disabled(),
        "a knowing click"
    );
}
