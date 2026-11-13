//! FEAT-0030 phase A on the demo core: the «Mixed» tile, «Mixed» as a source on Send with its
//! refusals and its excess, the coins not fully mixed in «Coins», the mixing lines in History.
//! The demo holds 1.00001 and twice 0.100001 DASH fully mixed, three coins of 0.0100001 not yet
//! (`evolite_demo::demo::MIXED_SPENDABLE`, `demo_coins`), and its plan burns by АРХ-9а.

use super::*;

/// Whether «Check and send» takes a click.
fn ready(kit: &Kit) -> bool {
    !button(kit, "Check and send").accesskit_node().is_disabled()
}

/// The form from «Mixed» to an address with `amount` duffs, once the core planned it.
fn mixed_form(amount: i64) -> (std::sync::MutexGuard<'static, ()>, Kit) {
    let (turn, mut kit) = demo_app("send-mixed", "en");
    kit.state_mut().send.amount = crate::format::input_text(amount);
    kit.state_mut().send.suggested = None;
    // The plan of this very amount: the preset's own may come first.
    wait_for(&mut kit, "the core's plan", |kit| {
        let m = &kit.state().send.mixed;
        m.asked.as_ref().is_some_and(|(_, a)| *a == amount) && m.quote.is_some()
    });
    kit.run_steps(3);
    (turn, kit)
}

/// AC-A1, AC-A11 (O1): the tile after Core, its words, a Tab stop of its own with its name, and
/// where its coins come from on focus.
#[test]
fn the_mixed_tile_stands_after_core_and_says_where_its_coins_are_from() {
    let (_turn, mut kit) = demo_app("overview-mixed", "en");
    wait_for(&mut kit, "the tile", |kit| has_text(kit, "Mixed"));
    assert!(
        has_words(&kit, "not mixed yet"),
        "the coins not mixed yet under it"
    );
    tab_to(&mut kit, "Mixed");
    assert_eq!(focused(&kit).as_deref(), Some("Mixed"));
    kit.run_steps(2);
    assert!(
        has_words(&kit, "Coins mixed in DashPay or Dash Core"),
        "the tip on Tab focus"
    );
}

/// AC-A2 (S1): «Mixed» is a source after Core; to an address the core's plan comes before the
/// button — «Mixed · 2 coins» and the excess going to the miners in the summary.
#[test]
fn a_payment_from_mixed_shows_the_coins_and_the_excess_before_the_password() {
    let (_turn, kit) = mixed_form(19_990_000);
    assert_eq!(
        kit.state()
            .send
            .mixed
            .quote
            .clone()
            .and_then(Result::ok)
            .map(|q| q.coins),
        Some(2)
    );
    assert!(
        has_text(&kit, "From: Mixed · 2 coins"),
        "the summary's «From»"
    );
    assert!(
        has_words(&kit, "Excess · to miners"),
        "the excess over dust in the summary"
    );
    assert!(has_words(&kit, "There is no change"), "the source's note");
    assert!(ready(&kit));
}

/// AC-A5 (S2): to a name or to Platform «Mixed» refuses before anything is asked; the button
/// stays grey.
#[test]
fn mixed_coins_go_only_to_a_core_address() {
    let (_turn, mut kit) = demo_app("send-mixed", "en");
    for (to, why) in [
        ("@alice", "only to a Core address"),
        (
            "tdash1qq7x4m9n2r8v5k3j6h0w4p2s8d5f7g9l3c0zq",
            "do not go to Platform",
        ),
    ] {
        kit.state_mut().send.recipient = to.into();
        kit.run_steps(4);
        assert!(has_words(&kit, why), "{to}: «{why}»");
        assert!(!ready(&kit), "{to}: the button grey");
    }
}

/// AC-A12 (S3): an excess over 0.01 DASH is refused under the amount with the amount that burns
/// nothing; «Use …» puts it in the field, and the core's plan for it lets the payment go on.
#[test]
fn a_too_large_excess_is_refused_with_the_amount_that_burns_nothing() {
    let (_turn, mut kit) = mixed_form(15_000_000);
    assert!(has_words(&kit, "is too large: mixed coins give no change"));
    assert!(!ready(&kit));
    let suggested = crate::format::amount(19_999_860);
    let use_it = format!("Use {suggested}");
    button(&kit, &use_it).click();
    kit.run_steps(2);
    assert_eq!(
        crate::format::parse_amount(&kit.state().send.amount),
        Some(19_999_860)
    );
    wait_for(&mut kit, "the plan of the amount suggested", |kit| {
        ready(kit)
    });
    assert!(!has_words(&kit, "is too large"));
}

/// AC-A3 (P2): an excess over 0.001 DASH is agreed to with a tick before «Sign and send»; any
/// change takes the tick off.
#[test]
fn a_large_excess_is_signed_only_with_the_tick() {
    let (_turn, mut kit) = mixed_form(19_500_000);
    assert!(ready(&kit));
    // As «Check and send» does (below the fold at 1024 × 768): from «Mixed» nothing else is asked.
    kit.state_mut().send.confirming = true;
    kit.run_steps(3);
    let sign = || "Sign and send";
    wait_for(&mut kit, "the last look", |kit| {
        kit.query_by_label(sign()).is_some()
    });
    assert!(has_words(&kit, "goes to the miners as part of the fee"));
    // 0.0049986 of 0.195: over 1 % and over 0.001 DASH, said in one line (P2).
    assert!(has_words(
        &kit,
        "The excess is over 1 % of the amount and over 0.001 DASH"
    ));
    assert!(
        button(&kit, sign()).accesskit_node().is_disabled(),
        "no tick"
    );
    assert!(
        kit.query_by_label("Change the amount").is_some(),
        "the way back is to the amount"
    );
    kit.query_all_by(|n| {
        n.label()
            .is_some_and(|l| l.starts_with("I understand: the excess"))
    })
    .next()
    .expect("the tick")
    .click();
    kit.run_steps(3);
    assert!(
        !button(&kit, sign()).accesskit_node().is_disabled(),
        "ticked"
    );
    // The core plans another excess (the snapshot changed): the tick goes.
    let plan = kit
        .state()
        .send
        .mixed
        .quote
        .clone()
        .and_then(Result::ok)
        .expect("a plan");
    assert_eq!(kit.state().send.mixed.agreed, Some(plan.excess), "ticked");
    let ask = kit.state().send.mixed.ask;
    let another = evolite_api::MixedQuote {
        excess: plan.excess + 1,
        ..plan
    };
    kit.state_mut().send.mixed.on_preview(ask, &Ok(another));
    assert_eq!(kit.state().send.mixed.agreed, None, "the tick goes");
}

/// AC-A6 (CC-M1, CC-M2): the coins not fully mixed stand apart with their rounds; alone they
/// may be applied with a warning, with ordinary ones not.
#[test]
fn coins_not_fully_mixed_go_only_alone() {
    let (_turn, mut kit) = demo_app("coins-mixed", "en");
    wait_for(&mut kit, "the coins", |k| {
        k.state().send.coins.coins.is_some()
    });
    kit.run_steps(3);
    assert!(has_words(&kit, "Not mixed yet"), "the group");
    assert!(has_words(&kit, "of 4 rounds"), "the rounds of a coin");
    assert!(has_words(&kit, "from the mixing account"));
    assert!(has_words(&kit, "part of their history can be traced"));
    assert!(!button(&kit, "Apply").accesskit_node().is_disabled());
    let (_turn2, mut kit) = {
        drop(kit);
        drop(_turn);
        demo_app("coins-mixed-refused", "en")
    };
    wait_for(&mut kit, "the coins", |k| {
        k.state().send.coins.coins.is_some()
    });
    kit.run_steps(3);
    assert!(has_words(&kit, "ordinary and not fully mixed"));
    assert!(has_words(&kit, "cannot be spent in one payment"));
    assert!(button(&kit, "Apply").accesskit_node().is_disabled());
}

/// AC-A7 (H1): the mixing lines under «Mixed», their own words, and a payment from mixed coins
/// named by its source.
#[test]
fn history_says_mixing_apart() {
    let (_turn, mut kit) = demo_app("activity-mixed", "en");
    wait_for(&mut kit, "the mixing lines", |kit| {
        has_words(kit, "Mixing (CoinJoin)")
    });
    assert!(has_words(&kit, "Into mixing"));
    assert!(has_words(
        &kit,
        "A mixing round: the balance did not change"
    ));
    assert!(has_words(
        &kit,
        "Between your own: coins split into denominations"
    ));
    assert!(has_words(&kit, "Mixed ·"), "the payment's source");
    assert!(
        !has_words(&kit, "Platform ·"),
        "only «Mixed» lines under its pill"
    );
}

/// AC-A11: in Arabic the tile and the source are there in the language's words, and the form
/// takes the plan.
#[test]
fn mixed_screens_open_in_arabic() {
    let (_turn, mut kit) = demo_app("overview-mixed", "ar");
    let mixed = crate::i18n::spoken("Mixed");
    assert_ne!(mixed, "Mixed", "translated");
    wait_for(&mut kit, "the tile", |kit| has_text(kit, &mixed));
    drop(kit);
    drop(_turn);
    let (_turn, mut kit) = demo_app("send-mixed", "ar");
    wait_for(&mut kit, "the plan", |kit| {
        kit.state().send.mixed.quote.is_some()
    });
    let s = &kit.state().send;
    assert_eq!((s.want, s.source), (None, 1), "«Mixed», after Core");
    assert!(s.mixed.quote.clone().is_some_and(|q| q.is_ok()));
}

/// R1: a wallet from DashPay with mixed coins has nothing to agree to — a note that they are
/// apart and spendable, no checkbox, «Set the password» on.
#[test]
fn a_dashpay_wallet_with_mixed_coins_goes_on_without_a_tick() {
    let (_turn, mut kit) = demo_app("dashpay", "en");
    wait_for(&mut kit, "the note", |kit| {
        has_words(kit, "Mixed coins will be shown apart")
    });
    assert!(!has_words(&kit, "continue without CoinJoin"));
    assert!(
        !button(&kit, "Set the password")
            .accesskit_node()
            .is_disabled()
    );
}

/// The designer's review, item 1: in ru the warning at the foot of «Coins for the payment» ran
/// under «Cancel». It wraps in the room left of the buttons, 16 from them.
#[test]
fn the_coins_warning_stays_clear_of_the_buttons() {
    for preset in ["coins-mixed", "coins-mixed-refused"] {
        let (turn, mut kit) = demo_app(preset, "ru");
        wait_for(&mut kit, "the coins", |k| {
            k.state().send.coins.coins.is_some()
        });
        kit.run_steps(3);
        // The line at the foot, the lowest saying it, and the dialog's «Cancel» beside it (the
        // form behind has one too).
        let note = kit
            .query_all_by(|n| {
                n.value().is_some_and(|v| {
                    v.contains("не до конца перемешанн") || v.contains("Не до конца перемешанн")
                })
            })
            .map(|n| n.rect())
            .max_by(|a, b| a.top().total_cmp(&b.top()))
            .unwrap_or_else(|| panic!("{preset}: the line at the foot"));
        let cancel_name = crate::i18n::spoken("Cancel");
        let cancel = kit
            .query_all_by_label(&cancel_name)
            .map(|n| n.rect())
            .find(|r| (r.center().y - note.center().y).abs() < 40.0)
            .expect("«Cancel» beside it");
        assert!(
            note.right() <= cancel.left() - 15.5,
            "{preset}: the line {note:?} runs into «Cancel» {cancel:?}"
        );
        drop(kit);
        drop(turn);
    }
}

/// The designer's review, item 4: denominations give long fractions — the tile shows four
/// decimals with «≈», and the whole amount is its description and tooltip (R4).
#[test]
fn the_mixed_tile_shows_four_decimals() {
    let (_turn, mut kit) = demo_app("overview-mixed", "en");
    wait_for(&mut kit, "the tile", |kit| has_text(kit, "≈ 1.2000"));
    let spendable = kit
        .state()
        .snapshot
        .as_ref()
        .and_then(|s| s.wallet.as_ref())
        .and_then(|w| w.balances.mixed)
        .expect("the demo's mixed coins")
        .spendable;
    let full = crate::format::amount(spendable);
    assert!(
        kit.query_all_by(|n| n.label().as_deref() == Some("≈ 1.2000"))
            .any(|n| n.accesskit_node().description().as_deref() == Some(full.as_str())),
        "the whole, {full}, said with the amount"
    );
    assert!(has_words(&kit, "not mixed yet ≈ 0.0300"));
}

/// The designer's review, item 5 (O1S): at 800 × 600 the tile's tip opened under Core and onto
/// the rail. It stands under its own tile, no wider.
#[test]
fn the_mixed_tip_stays_under_its_tile() {
    let (_turn, mut kit) = demo_app("overview-mixed", "en");
    kit.set_size(egui::vec2(800.0, 600.0));
    wait_for(&mut kit, "the tile", |kit| {
        kit.query_all_by_label("Mixed").next().is_some()
    });
    kit.run_steps(3);
    // The tile's own stop: the widest named so (its title is too).
    let tile = kit
        .query_all_by_label("Mixed")
        .map(|n| n.rect())
        .max_by(|a, b| a.width().total_cmp(&b.width()))
        .expect("the tile");
    kit.hover_at(tile.center());
    kit.run_steps(10);
    let tip = kit
        .query_all_by(|n| {
            n.value()
                .is_some_and(|v| v.starts_with("Coins mixed in DashPay"))
        })
        .map(|n| n.rect())
        .next()
        .expect("the tip on hover");
    assert!(
        tip.left() >= tile.left() - 0.5 && tip.right() <= tile.right() + 0.5,
        "the tip {tip:?} under its tile {tile:?}"
    );
}

/// The words of a label that says `part`, the first found.
fn said(kit: &Kit, part: &str) -> Option<egui::Rect> {
    kit.query_all_by(|n| n.value().is_some_and(|v| v.contains(part)))
        .map(|n| n.rect())
        .next()
}

/// The designer's round 2, item 2: at 800 × 440 the last look's middle is brought to the excess
/// block of P2 — the tick says nothing without it — while P1, nothing to agree to, opens at its
/// top as every dialog does (BUG-122).
#[test]
fn a_low_last_look_shows_the_excess_to_agree_to() {
    use crate::components::dialog_seen;
    for (preset, brought) in [("send-mixed-agree", true), ("send-mixed-confirm", false)] {
        let (turn, mut kit) = demo_app(preset, "en");
        kit.set_size(egui::vec2(800.0, 440.0));
        // As the last look opens at this size.
        kit.state_mut().send.mixed.revealed = None;
        wait_for(&mut kit, "the block", |kit| {
            said(kit, "goes to the miners as part of the fee").is_some()
                && dialog_seen(&kit.ctx).is_some_and(|d| d.scrolled.is_some())
        });
        kit.run_steps(10);
        let dialog = dialog_seen(&kit.ctx).expect("the last look");
        let view = dialog.scrolled.expect("its middle scrolls at 800 × 440");
        let block = said(&kit, "goes to the miners as part of the fee").expect("the block");
        if brought {
            // Its first line, the reason, in view (the middle is lower than the block here).
            assert!(
                view.expand(1.0).contains_rect(block),
                "{preset}: the block's first line {block:?} in the middle {view:?}"
            );
            assert!(dialog.offset > 0.0, "{preset}: brought down to it");
        } else {
            assert_eq!(dialog.offset, 0.0, "{preset}: at its top");
        }
        drop(kit);
        drop(turn);
    }
}

/// The designer's round 2, item 3: at 800 × 440 the refusal of a too large excess comes into
/// view with its link (S3).
#[test]
fn a_low_form_brings_the_refusal_into_view() {
    let (_turn, mut kit) = demo_app("send-mixed-refused", "en");
    kit.set_size(egui::vec2(800.0, 440.0));
    kit.state_mut().send.mixed.refusal_revealed = None;
    let link = format!("Use {}", crate::format::amount(19_999_860));
    wait_for(&mut kit, "the link", |kit| {
        kit.query_all_by_label(&link).next().is_some()
    });
    kit.run_steps(10);
    let window = kit.ctx.content_rect();
    let rect = kit
        .query_all_by_label(&link)
        .next()
        .expect("the link")
        .rect();
    assert!(
        window.contains_rect(rect),
        "the link {rect:?} in the window {window:?}"
    );
}

/// The designer's round 2, the small one: from «Mixed» to a name the reason is under «To», not
/// under «Amount» (S2).
#[test]
fn a_name_is_refused_under_to() {
    let (_turn, mut kit) = demo_app("send-mixed-name", "en");
    wait_for(&mut kit, "the reason", |kit| {
        said(kit, "only to a Core address").is_some()
    });
    let reason = said(&kit, "only to a Core address").expect("the reason");
    let amount = kit
        .query_all_by(|n| n.value().as_deref() == Some("Amount"))
        .map(|n| n.rect())
        .next()
        .expect("«Amount»");
    assert!(
        reason.bottom() <= amount.top(),
        "the reason {reason:?} over «Amount» {amount:?}"
    );
}
